//! Typed fixture import plans.
//!
//! The SQL mirrors the old importer’s important invariants: prefix-column
//! COPY for schema lag, stable file/clip identities, session and audio remaps,
//! natural-key stamp/event deduplication, and transactional pruning.

use anyhow::{Result, bail};
use async_trait::async_trait;

use crate::db::{CopySection, LocalDatabase, SqlBatch, quote_identifier};
use crate::fixtures::remote::RemoteSql;
use crate::fixtures::workspace::FixtureWorkspace;

#[async_trait]
pub trait ColumnProvider: Send + Sync {
    async fn table_columns(&self, table: &str) -> Result<Vec<String>>;
}

#[async_trait]
impl ColumnProvider for dyn LocalDatabase {
    async fn table_columns(&self, table: &str) -> Result<Vec<String>> {
        LocalDatabase::table_columns(self, table).await
    }
}

#[async_trait]
impl<R: crate::runner::CommandRunner + ?Sized> ColumnProvider for RemoteSql<'_, R> {
    async fn table_columns(&self, table: &str) -> Result<Vec<String>> {
        RemoteSql::table_columns(self, table)
    }
}

pub async fn build_import_batch<P: ColumnProvider + ?Sized>(
    provider: &P,
    workspace: &FixtureWorkspace,
    marker: Option<&str>,
) -> Result<SqlBatch> {
    let mut batch = SqlBatch::default();

    for (table, file, fixup) in [
        (
            "audio_file_finalize_reasons",
            "audio_file_finalize_reasons.tsv",
            "",
        ),
        ("user_name_event_types", "user_name_event_types.tsv", ""),
        ("channel_type", "channel_type.tsv", ""),
        ("guilds", "guilds.tsv", ""),
        (
            "roles",
            "roles.tsv",
            "DELETE FROM roles WHERE guild_id IN (SELECT id FROM _import_guild_ids);",
        ),
        (
            "channels",
            "channels.tsv",
            "DELETE FROM channels WHERE guild_id IN (SELECT id FROM _import_guild_ids);",
        ),
        (
            "channel_permissions",
            "channel_permissions.tsv",
            "DELETE FROM channel_permissions cp USING channels c WHERE cp.channel_id = c.channel_id AND c.guild_id IN (SELECT id FROM _import_guild_ids);",
        ),
        ("user_roles", "user_roles.tsv", ""),
        ("user_names", "user_names.tsv", ""),
        ("user_nicknames", "user_nicknames.tsv", ""),
        ("user_name_history", "user_name_history.tsv", ""),
    ] {
        if workspace.path(file).is_file() {
            append_table_copy(provider, &mut batch, workspace, table, file, fixup, None).await?;
        }
    }

    if workspace.path("guilds.tsv").is_file() {
        let guilds = workspace.read_text("guilds.tsv")?;
        let data = guilds
            .lines()
            .filter_map(|line| line.split('\t').next())
            .filter(|id| !id.is_empty())
            .map(|id| format!("{id}\n"))
            .collect::<String>();
        batch.copies.push(CopySection {
            statement: "COPY _import_guild_ids FROM STDIN".into(),
            data: data.into_bytes(),
        });
    }
    // Lookup copies above use _import_guild_ids in their fixups. The temp table
    // must exist before those copies are run, so move its declaration to the
    // beginning once all sections have been collected.
    let guild_declaration =
        "CREATE TEMP TABLE _import_guild_ids (id bigint PRIMARY KEY) ON COMMIT DROP;\n";
    batch.setup = format!("{guild_declaration}{}", batch.setup);

    append_table_copy(
        provider,
        &mut batch,
        workspace,
        "recording_sessions",
        "recording_sessions.tsv",
        "",
        Some("_imp_sessions"),
    )
    .await?;
    append_table_copy(
        provider,
        &mut batch,
        workspace,
        "audio_files",
        "audio_files.tsv",
        "",
        Some("_imp_audio"),
    )
    .await?;
    append_table_copy(
        provider,
        &mut batch,
        workspace,
        "recording_gaps",
        "recording_gaps.tsv",
        "",
        Some("_imp_gaps"),
    )
    .await?;
    append_table_copy(
        provider,
        &mut batch,
        workspace,
        "recording_session_events",
        "recording_session_events.tsv",
        "",
        Some("_imp_events"),
    )
    .await?;
    append_table_copy(
        provider,
        &mut batch,
        workspace,
        "clips",
        "clips.tsv",
        "",
        Some("_imp_clips"),
    )
    .await?;
    append_table_copy(
        provider,
        &mut batch,
        workspace,
        "stamps",
        "stamps.tsv",
        "",
        Some("_imp_stamps"),
    )
    .await?;
    append_table_copy(
        provider,
        &mut batch,
        workspace,
        "voice_state_event_types",
        "voice_state_event_types.tsv",
        "",
        Some("_imp_vse_types"),
    )
    .await?;
    append_table_copy(
        provider,
        &mut batch,
        workspace,
        "voice_state_events",
        "voice_state_events.tsv",
        "",
        Some("_imp_vse"),
    )
    .await?;
    let import_voice_connection = workspace.path("voice_connection_events.tsv").is_file()
        && !provider
            .table_columns("voice_connection_events")
            .await?
            .is_empty();
    if import_voice_connection {
        append_table_copy(
            provider,
            &mut batch,
            workspace,
            "voice_connection_events",
            "voice_connection_events.tsv",
            "",
            Some("_imp_vce"),
        )
        .await?;
    }

    append_selection_transform(&mut batch, marker, import_voice_connection).await?;
    Ok(batch)
}

pub async fn build_prune_batch<P: ColumnProvider + ?Sized>(
    provider: &P,
    keep: &[String],
    managed: &[String],
) -> Result<SqlBatch> {
    let mut batch = SqlBatch::default();
    batch.setup.push_str(
        "CREATE TEMP TABLE _keep (file_name text PRIMARY KEY) ON COMMIT DROP;\n\
         CREATE TEMP TABLE _managed (file_name text PRIMARY KEY) ON COMMIT DROP;\n",
    );
    batch.copies.push(CopySection {
        statement: "COPY _keep FROM STDIN".into(),
        data: lines(keep),
    });
    batch.copies.push(CopySection {
        statement: "COPY _managed FROM STDIN".into(),
        data: lines(managed),
    });
    batch.finish.push_str(
        "CREATE TEMP TABLE _touched_sessions ON COMMIT DROP AS
             SELECT DISTINCT af.recording_session_id AS id
               FROM audio_files af
               JOIN _managed m ON m.file_name = af.file_name
              WHERE af.recording_session_id IS NOT NULL;
         DELETE FROM media_objects mo
          USING audio_files af, _managed m
          WHERE mo.audio_file_id = af.id
            AND af.file_name = m.file_name
            AND NOT EXISTS (SELECT 1 FROM _keep k WHERE k.file_name = af.file_name);
         DELETE FROM audio_files af
          USING _managed m
          WHERE af.file_name = m.file_name
            AND NOT EXISTS (SELECT 1 FROM _keep k WHERE k.file_name = af.file_name);
         DELETE FROM recording_sessions rs
          USING _touched_sessions t
          WHERE rs.id = t.id
            AND NOT EXISTS (SELECT 1 FROM audio_files af WHERE af.recording_session_id = rs.id)
            AND NOT EXISTS (SELECT 1 FROM clips c WHERE c.recording_session_id = rs.id)
            AND NOT EXISTS (SELECT 1 FROM stamps st WHERE st.recording_session_id = rs.id);
        ",
    );
    // Validate that pruning's current FK order is available before a destructive
    // batch is sent. The provider also makes this function useful in scripted
    // tests without connecting to PostgreSQL.
    if provider.table_columns("audio_files").await?.is_empty() {
        bail!("destination does not contain audio_files; run migrations first")
    }
    Ok(batch)
}

async fn append_table_copy<P: ColumnProvider + ?Sized>(
    provider: &P,
    batch: &mut SqlBatch,
    workspace: &FixtureWorkspace,
    table: &str,
    file: &str,
    fixup: &str,
    temp_name: Option<&str>,
) -> Result<()> {
    let temp = temp_name
        .map(str::to_string)
        .unwrap_or_else(|| format!("_imp_{}", table.replace('_', "")));
    let columns = provider.table_columns(table).await?;
    if columns.is_empty() {
        // voice_connection_events was removed by a migration. An old export
        // may still contain it, but the destination must not be made to fail
        // merely because that optional source table existed.
        if table == "voice_connection_events" {
            return Ok(());
        }
        if workspace.path(file).is_file() {
            bail!("destination table {table} does not exist")
        }
        return Ok(());
    }
    batch.setup.push_str(&format!(
        "CREATE TEMP TABLE {temp} (LIKE {} INCLUDING DEFAULTS) ON COMMIT DROP;\n",
        quote_identifier(table)?
    ));
    let data = if workspace.path(file).is_file() {
        workspace.read(file)?
    } else {
        Vec::new()
    };
    if !data.is_empty() {
        let source_columns = data
            .split(|byte| *byte == b'\n')
            .find(|line| !line.is_empty())
            .map(|line| line.split(|byte| *byte == b'\t').count())
            .unwrap_or(0);
        if source_columns > columns.len() {
            bail!(
                "{} export has {} columns but destination has only {}",
                table,
                source_columns,
                columns.len()
            )
        }
        let statement = if source_columns == columns.len() {
            format!("COPY {temp} FROM STDIN")
        } else {
            let prefix = columns
                .iter()
                .take(source_columns)
                .map(|column| quote_identifier(column))
                .collect::<Result<Vec<_>>>()?
                .join(", ");
            format!("COPY {temp} ({prefix}) FROM STDIN")
        };
        batch.copies.push(CopySection { statement, data });
    }
    if !fixup.is_empty() {
        batch.finish.push_str(fixup);
        batch.finish.push('\n');
    }
    if temp_name.is_none() {
        batch.finish.push_str(&format!(
            "INSERT INTO {} SELECT * FROM {temp} ON CONFLICT DO NOTHING;\n",
            quote_identifier(table)?
        ));
    }
    Ok(())
}

async fn append_selection_transform(
    batch: &mut SqlBatch,
    marker: Option<&str>,
    import_voice_connection: bool,
) -> Result<()> {
    // Temp tables are deliberately named after the source tables so the SQL is
    // readable in a failed import log. The copy sections are appended here
    // rather than using INSERT statements, preserving PostgreSQL COPY speed.
    let mut selection = String::new();
    selection.push_str(
        "UPDATE _imp_sessions SET owner_instance_id = NULL;
         UPDATE _imp_audio
            SET recording_owner_instance_id = NULL,
                recording_heartbeat_at = NULL;
         CREATE TEMP TABLE _session_map (
             old_id bigint PRIMARY KEY,
             new_id bigint NOT NULL,
             reused boolean NOT NULL DEFAULT false
         ) ON COMMIT DROP;
         INSERT INTO _session_map (old_id, new_id, reused)
              SELECT DISTINCT ON (a.recording_session_id)
                     a.recording_session_id, existing.recording_session_id, true
                FROM _imp_audio a
                JOIN audio_files existing ON existing.file_name = a.file_name
               WHERE a.recording_session_id IS NOT NULL
                 AND existing.recording_session_id IS NOT NULL
              ON CONFLICT (old_id) DO NOTHING;
         INSERT INTO _session_map (old_id, new_id, reused)
              SELECT DISTINCT ON (c.recording_session_id)
                     c.recording_session_id, existing.recording_session_id, true
                FROM _imp_clips c
                JOIN clips existing ON existing.clip_id = c.clip_id
               WHERE c.recording_session_id IS NOT NULL
                 AND existing.recording_session_id IS NOT NULL
              ON CONFLICT (old_id) DO NOTHING;
         CREATE TEMP TABLE _audio_map (
             old_id bigint PRIMARY KEY,
             new_id bigint NOT NULL
         ) ON COMMIT DROP;
         INSERT INTO _audio_map (old_id, new_id)
              SELECT a.id, existing.id
                FROM _imp_audio a
                JOIN audio_files existing ON existing.file_name = a.file_name
              ON CONFLICT (old_id) DO NOTHING;
         DELETE FROM _imp_audio a USING audio_files existing
               WHERE existing.file_name = a.file_name;
         DELETE FROM _imp_sessions s
               WHERE EXISTS (SELECT 1 FROM _session_map m WHERE m.old_id = s.id)
                  OR (NOT EXISTS (SELECT 1 FROM _imp_audio a WHERE a.recording_session_id = s.id)
                      AND NOT EXISTS (SELECT 1 FROM _imp_clips c WHERE c.recording_session_id = s.id)
                      AND NOT EXISTS (SELECT 1 FROM _imp_stamps st WHERE st.recording_session_id = s.id));
         SELECT setval(pg_get_serial_sequence('public.recording_sessions', 'id'),
                       GREATEST((SELECT COALESCE(MAX(id), 0) FROM recording_sessions),
                                (SELECT COALESCE(MAX(id), 0) FROM _imp_sessions), 1));
         INSERT INTO _session_map (old_id, new_id)
              SELECT s.id,
                     CASE WHEN EXISTS (SELECT 1 FROM recording_sessions d WHERE d.id = s.id)
                          THEN nextval(pg_get_serial_sequence('public.recording_sessions', 'id'))
                          ELSE s.id END
                FROM _imp_sessions s;
         DELETE FROM _imp_gaps g
               WHERE NOT EXISTS (SELECT 1 FROM _imp_sessions s WHERE s.id = g.recording_session_id);
         DELETE FROM _imp_events e
               WHERE NOT EXISTS (SELECT 1 FROM _imp_sessions s WHERE s.id = e.recording_session_id);
         UPDATE _imp_audio a SET recording_session_id = m.new_id
                FROM _session_map m WHERE m.old_id = a.recording_session_id;
         UPDATE _imp_gaps g SET recording_session_id = m.new_id
                FROM _session_map m WHERE m.old_id = g.recording_session_id;
         UPDATE _imp_events e SET recording_session_id = m.new_id
                FROM _session_map m WHERE m.old_id = e.recording_session_id;
         UPDATE _imp_sessions s SET id = m.new_id
                FROM _session_map m WHERE m.old_id = s.id;
         CREATE TEMP TABLE _audio_new ON COMMIT DROP AS
              SELECT id AS old_id,
                     nextval(pg_get_serial_sequence('public.audio_files', 'id')) AS new_id
                FROM _imp_audio;
         INSERT INTO _audio_map (old_id, new_id)
              SELECT old_id, new_id FROM _audio_new ON CONFLICT (old_id) DO NOTHING;
         UPDATE _imp_audio a SET id = n.new_id FROM _audio_new n WHERE n.old_id = a.id;
         UPDATE _imp_gaps SET id = nextval(pg_get_serial_sequence('public.recording_gaps', 'id'));
         UPDATE _imp_events SET id = nextval(pg_get_serial_sequence('public.recording_session_events', 'id'));
         INSERT INTO recording_sessions SELECT * FROM _imp_sessions;
         SELECT setval(pg_get_serial_sequence('public.recording_sessions', 'id'),
                       GREATEST((SELECT COALESCE(MAX(id), 0) FROM recording_sessions), 1));
         INSERT INTO audio_files SELECT * FROM _imp_audio;
         INSERT INTO recording_gaps SELECT * FROM _imp_gaps;
         INSERT INTO recording_session_events SELECT * FROM _imp_events;
         UPDATE _imp_clips c
            SET recording_session_id =
                (SELECT m.new_id FROM _session_map m WHERE m.old_id = c.recording_session_id)
          WHERE c.recording_session_id IS NOT NULL;
         DELETE FROM _imp_clips c USING clips existing WHERE existing.clip_id = c.clip_id;
         INSERT INTO clips SELECT * FROM _imp_clips;
         DELETE FROM _imp_stamps s USING stamps existing
               WHERE existing.guild_id = s.guild_id
                 AND existing.channel_id = s.channel_id
                 AND existing.target_user_id = s.target_user_id
                 AND existing.stamper_user_id = s.stamper_user_id
                 AND existing.stamp_ts = s.stamp_ts;
         UPDATE _imp_stamps s
            SET recording_session_id =
                (SELECT m.new_id FROM _session_map m WHERE m.old_id = s.recording_session_id)
          WHERE s.recording_session_id IS NOT NULL;
         UPDATE _imp_stamps s
            SET audio_file_id =
                (SELECT m.new_id FROM _audio_map m WHERE m.old_id = s.audio_file_id)
          WHERE s.audio_file_id IS NOT NULL;
         UPDATE _imp_stamps SET id = nextval(pg_get_serial_sequence('public.stamps', 'id'));
         INSERT INTO stamps SELECT * FROM _imp_stamps;
         INSERT INTO voice_state_event_types (name)
              SELECT t.name FROM _imp_vse_types t
               WHERE NOT EXISTS (
                     SELECT 1 FROM voice_state_event_types d WHERE d.name = t.name);
         UPDATE _imp_vse v SET event_type_id = d.id
                FROM _imp_vse_types t
                JOIN voice_state_event_types d ON d.name = t.name
               WHERE t.id = v.event_type_id;
         DELETE FROM _imp_vse v USING voice_state_events e
               WHERE e.guild_id = v.guild_id AND e.user_id = v.user_id
                 AND e.event_type_id = v.event_type_id AND e.occurred_at = v.occurred_at;
         UPDATE _imp_vse SET id = nextval(pg_get_serial_sequence('public.voice_state_events', 'id'));
         INSERT INTO voice_state_events SELECT * FROM _imp_vse;
        ",
    );
    if import_voice_connection {
        selection.push_str(
            "UPDATE _imp_vce SET owner_instance_id = NULL;
             DELETE FROM _imp_vce v USING voice_connection_events e
                    WHERE e.operation_id = v.operation_id;
             UPDATE _imp_vce SET id = nextval(pg_get_serial_sequence('public.voice_connection_events', 'id'));
             INSERT INTO voice_connection_events SELECT * FROM _imp_vce;
            ",
        );
    }
    if let Some(marker) = marker {
        selection.push_str(&format!(
            "INSERT INTO recording_session_events
                 (recording_session_id, occurred_at, event_type, channel_id, details)
             SELECT rs.id, rs.started_at, 'imported_from_production', rs.starting_channel_id,
                    {}::jsonb || jsonb_build_object('origin_session_id', m.old_id)
               FROM _session_map m
               JOIN recording_sessions rs ON rs.id = m.new_id
              WHERE NOT EXISTS (
                    SELECT 1 FROM recording_session_events e
                     WHERE e.recording_session_id = rs.id
                       AND e.event_type = 'imported_from_production');
            ",
            sql_literal(marker)
        ));
    }
    batch.finish.push_str(&selection);
    Ok(())
}

fn lines(values: &[String]) -> Vec<u8> {
    if values.is_empty() {
        Vec::new()
    } else {
        format!("{}\n", values.join("\n")).into_bytes()
    }
}

fn sql_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    struct TestColumns;

    #[async_trait]
    impl ColumnProvider for TestColumns {
        async fn table_columns(&self, _table: &str) -> Result<Vec<String>> {
            Ok(vec!["id".into()])
        }
    }

    #[test]
    fn sql_literals_are_not_shell_literals() {
        assert_eq!(
            sql_literal("{\"source\":\"production\"}"),
            "'{\"source\":\"production\"}'"
        );
    }

    #[tokio::test]
    async fn import_declares_guild_lookup_table_once() {
        let workspace = FixtureWorkspace::new().unwrap();
        workspace.write("guilds.tsv", "42\n").unwrap();

        let batch = build_import_batch(&TestColumns, &workspace, None)
            .await
            .unwrap();
        let rendered = String::from_utf8(batch.render_psql()).unwrap();

        assert_eq!(
            rendered
                .matches("CREATE TEMP TABLE _import_guild_ids")
                .count(),
            1
        );
    }
}
