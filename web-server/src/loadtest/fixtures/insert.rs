//! Writing a planned fixture: the guild, roster, roles and permissions, then
//! its recording history, in one transaction.

use super::*;

pub(super) async fn insert_all(
    state: &LoadtestState,
    spec: &FixtureSpec,
    sources: &Sources,
    plan: &FixturePlan,
) -> Result<(), AppError> {
    let FixturePlan {
        guild_id,
        ref members,
        ref channels,
        public,
        ref fragments,
        ref clips,
        ref stamps,
    } = *plan;
    let mut tx = state.pool.begin().await?;
    let owner = members
        .first()
        .map(|member| member.id)
        .ok_or(AppError::InternalError)?;

    super::super::agent::register(&mut tx, &state.agent_id).await?;
    sqlx::query("INSERT INTO guilds (id, owner_id, name, icon) VALUES ($1, $2, $3, NULL)")
        .bind(guild_id)
        .bind(owner)
        .bind(format!(
            "Load test {}",
            ids::guild_of(guild_id).unwrap_or(0)
        ))
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO guild_projection_state
            (guild_id, owner_instance_id, generation, roster_complete_at, presence_synced_at)
         VALUES ($1, $2, 1, now(), now())",
    )
    .bind(guild_id)
    .bind(&state.agent_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO guilds_present (guild_id) VALUES ($1) ON CONFLICT DO NOTHING")
        .bind(guild_id)
        .execute(&mut *tx)
        .await?;

    // Roles: @everyone (id = guild id), Insiders, Moderators, then cosmetic.
    let mut role_ids = vec![guild_id];
    let mut role_names = vec!["@everyone".to_string()];
    let mut role_perms = vec![EVERYONE_PERMISSIONS];
    for index in 0..FUNCTIONAL_ROLES + spec.cosmetic_roles {
        role_ids.push(ids::role_id(guild_id, index));
        role_names.push(match index {
            INSIDERS => "Insiders".to_string(),
            MODERATORS => "Moderators".to_string(),
            _ => format!("Role {index}"),
        });
        role_perms.push(if index == MODERATORS { MANAGE_GUILD } else { 0 });
    }
    sqlx::query(
        "INSERT INTO roles (guild_id, role_id, permission, name, color)
         SELECT $1, * FROM UNNEST($2::bigint[], $3::bigint[], $4::text[], $5::bigint[])",
    )
    .bind(guild_id)
    .bind(&role_ids)
    .bind(&role_perms)
    .bind(&role_names)
    .bind(
        role_ids
            .iter()
            .map(|id| id % 0xFF_FFFF)
            .collect::<Vec<i64>>(),
    )
    .execute(&mut *tx)
    .await?;

    let channel_names: Vec<String> = (0..channels.len())
        .map(|index| {
            if index < public {
                format!("voice-{}", index + 1)
            } else {
                format!("insiders-{}", index + 1 - public)
            }
        })
        .collect();
    sqlx::query(
        "INSERT INTO channels (channel_id, guild_id, type, name)
         SELECT channel_id, $1, $2, name FROM UNNEST($3::bigint[], $4::text[]) AS c(channel_id, name)",
    )
    .bind(guild_id)
    .bind(VOICE_CHANNEL)
    .bind(channels)
    .bind(&channel_names)
    .execute(&mut *tx)
    .await?;
    let restricted: Vec<i64> = channels[public..].to_vec();
    sqlx::query(
        "INSERT INTO channel_permissions (channel_id, target_id, kind, allow, deny)
         SELECT channel_id, $2, 'role', 0, $3 FROM UNNEST($1::bigint[]) AS c(channel_id)
         UNION ALL
         SELECT channel_id, $4, 'role', $5, 0 FROM UNNEST($1::bigint[]) AS c(channel_id)",
    )
    .bind(&restricted)
    .bind(guild_id)
    .bind(VIEW_CHANNEL)
    .bind(ids::role_id(guild_id, INSIDERS))
    .bind(VIEW_CHANNEL | CONNECT)
    .execute(&mut *tx)
    .await?;

    let member_ids: Vec<i64> = members.iter().map(|member| member.id).collect();
    let usernames: Vec<String> = member_ids
        .iter()
        .enumerate()
        .map(|(index, _)| format!("lt-user-{index}"))
        .collect();
    sqlx::query(
        "INSERT INTO guild_members (guild_id, user_id, username, global_name)
         SELECT $1, user_id, username, 'Load Tester ' || username
           FROM UNNEST($2::bigint[], $3::text[]) AS m(user_id, username)",
    )
    .bind(guild_id)
    .bind(&member_ids)
    .bind(&usernames)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO user_names (user_id, username, global_name)
         SELECT user_id, username, 'Load Tester ' || username
           FROM UNNEST($1::bigint[], $2::text[]) AS m(user_id, username)
         ON CONFLICT (user_id) DO NOTHING",
    )
    .bind(&member_ids)
    .bind(&usernames)
    .execute(&mut *tx)
    .await?;
    // The agent records names as it first sees them, before their
    // recordings; listings resolve names from this history. The kinds are
    // the agent's `UserNameEventType`; a database copied from staging has
    // them already, a fresh one does not.
    sqlx::query(
        "INSERT INTO user_name_event_types (id, name)
         VALUES (1, 'username'), (2, 'global_name'), (3, 'nickname')
         ON CONFLICT DO NOTHING",
    )
    .execute(&mut *tx)
    .await?;
    let mut history_ids = Vec::with_capacity(member_ids.len() * 2);
    let mut history_users = Vec::with_capacity(member_ids.len() * 2);
    let mut history_kinds = Vec::with_capacity(member_ids.len() * 2);
    let mut history_values = Vec::with_capacity(member_ids.len() * 2);
    for (index, (user_id, username)) in member_ids.iter().zip(&usernames).enumerate() {
        let index = u32::try_from(index).map_err(|_| AppError::InternalError)?;
        for (kind, value) in [
            (1, username.clone()),
            (2, format!("Load Tester {username}")),
        ] {
            history_ids.push(ids::name_history_id(guild_id, index, kind));
            history_users.push(*user_id);
            history_kinds.push(kind);
            history_values.push(value);
        }
    }
    sqlx::query(
        "INSERT INTO user_name_history (id, user_id, guild_id, kind_id, value, observed_at)
         SELECT id, user_id, NULL, kind_id, value, now() - make_interval(days => $5 + 1)
           FROM UNNEST($1::bigint[], $2::bigint[], $3::int[], $4::text[])
                AS h(id, user_id, kind_id, value)",
    )
    .bind(&history_ids)
    .bind(&history_users)
    .bind(&history_kinds)
    .bind(&history_values)
    .bind(i32::try_from(spec.history_days).map_err(|_| AppError::InternalError)?)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO discord_auth_user (id, username, avatar)
         SELECT id, username, '' FROM UNNEST($1::bigint[], $2::text[]) AS m(id, username)
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(&member_ids)
    .bind(&usernames)
    .execute(&mut *tx)
    .await?;

    let mut rng = fastrand::Rng::with_seed(spec.seed.wrapping_add(17));
    let mut grant_users = Vec::new();
    let mut grant_roles = Vec::new();
    for (index, member) in members.iter().enumerate() {
        let mut roles = Vec::new();
        if member.insider {
            roles.push(ids::role_id(guild_id, INSIDERS));
        }
        if (1..=MODERATOR_COUNT as usize).contains(&index) {
            roles.push(ids::role_id(guild_id, MODERATORS));
        }
        if spec.cosmetic_roles > 0 {
            for _ in 0..rng.u32(0..=2) {
                let role =
                    ids::role_id(guild_id, FUNCTIONAL_ROLES + rng.u32(0..spec.cosmetic_roles));
                if !roles.contains(&role) {
                    roles.push(role);
                }
            }
        }
        for role in roles {
            grant_users.push(member.id);
            grant_roles.push(role);
        }
    }
    sqlx::query("INSERT INTO user_roles (user_id, role_id) SELECT * FROM UNNEST($1::bigint[], $2::bigint[])")
        .bind(&grant_users)
        .bind(&grant_roles)
        .execute(&mut *tx)
        .await?;

    insert_history(&mut tx, sources, guild_id, fragments, clips, stamps).await?;
    tx.commit().await?;
    Ok(())
}

async fn insert_history(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    sources: &Sources,
    guild_id: i64,
    fragments: &[Fragment],
    clips: &[Clip],
    stamps: &[Stamp],
) -> Result<(), AppError> {
    let session_ids: Vec<i64> = fragments.iter().map(|f| f.session_id).collect();
    let audio_ids: Vec<i64> = fragments.iter().map(|f| f.audio_file_id).collect();
    let users: Vec<i64> = fragments.iter().map(|f| f.user_id).collect();
    let channels: Vec<i64> = fragments.iter().map(|f| f.channel_id).collect();
    let starts: Vec<i64> = fragments.iter().map(|f| f.start_ms).collect();
    let ends: Vec<i64> = fragments.iter().map(|f| f.end_ms).collect();
    let keys: Vec<sakiot_paths::RecordingKey> = fragments
        .iter()
        .map(|f| recording_key(guild_id, f))
        .collect();
    let stems: Vec<String> = keys.iter().map(|key| key.stem.clone()).collect();
    let years: Vec<i32> = keys.iter().map(|key| key.year).collect();
    let months: Vec<i32> = keys.iter().map(|key| key.month as i32).collect();

    // A finished sitting as the recorder leaves it: the member left, the
    // grace period ran out, and the session finalized at the leave time.
    sqlx::query(
        "INSERT INTO recording_sessions
            (id, guild_id, user_id, starting_channel_id, current_channel_id, state,
             started_at, ended_at, pause_started_at, end_reason, last_segment_index)
         SELECT id, $1, user_id, channel_id, channel_id, 'finalized',
                to_timestamp(start_ms / 1000.0), to_timestamp(end_ms / 1000.0),
                to_timestamp(end_ms / 1000.0), 'pending_grace_expired', 0
           FROM UNNEST($2::bigint[], $3::bigint[], $4::bigint[], $5::bigint[], $6::bigint[])
             AS s(id, user_id, channel_id, start_ms, end_ms)",
    )
    .bind(guild_id)
    .bind(&session_ids)
    .bind(&users)
    .bind(&channels)
    .bind(&starts)
    .bind(&ends)
    .execute(&mut **tx)
    .await?;

    sqlx::query(
        "INSERT INTO audio_files
            (id, file_name, guild_id, channel_id, user_id, year, month, start_ts, end_ts,
             recording_session_id, segment_index, finalize_reason_id)
         SELECT id, file_name, $1, channel_id, user_id, year, month, start_ms, end_ms,
                session_id, 0, $2
           FROM UNNEST($3::bigint[], $4::text[], $5::bigint[], $6::bigint[], $7::int[],
                       $8::int[], $9::bigint[], $10::bigint[], $11::bigint[])
             AS a(id, file_name, channel_id, user_id, year, month, start_ms, end_ms, session_id)",
    )
    .bind(guild_id)
    .bind(FINALIZE_WRITER_CLOSE)
    .bind(&audio_ids)
    .bind(&stems)
    .bind(&channels)
    .bind(&users)
    .bind(&years)
    .bind(&months)
    .bind(&starts)
    .bind(&ends)
    .bind(&session_ids)
    .execute(&mut **tx)
    .await?;

    let mut event_sessions = Vec::new();
    let mut event_at = Vec::new();
    let mut event_types = Vec::new();
    let mut event_channels = Vec::new();
    let mut event_details = Vec::new();
    for (index, f) in fragments.iter().enumerate() {
        let opened = serde_json::json!({
            "audio_file_id": f.audio_file_id, "segment_index": 0, "file_name": stems[index],
        });
        let closed = serde_json::json!({
            "audio_file_id": f.audio_file_id, "segment_index": 0, "reason": "writer_close",
        });
        let left = serde_json::json!({
            "reason": "disconnect", "pending_deadline_ms": f.end_ms + PENDING_GRACE_MS,
            "absolute_cap_deadline_ms": null,
        });
        let expired = serde_json::json!({
            "ended_at_ms": f.end_ms, "end_reason": "pending_grace_expired",
        });
        for (at, kind, details) in [
            (f.start_ms, "session_start", serde_json::json!({})),
            (f.start_ms, "fragment_open", opened),
            (f.end_ms, "fragment_close", closed),
            (f.end_ms, "disconnect", left),
            (f.end_ms + PENDING_GRACE_MS, "timeout", expired),
        ] {
            event_sessions.push(f.session_id);
            event_at.push(at);
            event_types.push(kind.to_string());
            event_channels.push(f.channel_id);
            event_details.push(details.to_string());
        }
    }
    sqlx::query(
        "INSERT INTO recording_session_events
            (recording_session_id, occurred_at, event_type, channel_id, details)
         SELECT session_id, to_timestamp(at_ms / 1000.0), event_type, channel_id, details::jsonb
           FROM UNNEST($1::bigint[], $2::bigint[], $3::text[], $4::bigint[], $5::text[])
             AS e(session_id, at_ms, event_type, channel_id, details)",
    )
    .bind(&event_sessions)
    .bind(&event_at)
    .bind(&event_types)
    .bind(&event_channels)
    .bind(&event_details)
    .execute(&mut **tx)
    .await?;

    if !clips.is_empty() {
        let mut clip_ids = Vec::new();
        let mut lengths = Vec::new();
        let mut sizes = Vec::new();
        let mut clip_channels = Vec::new();
        let mut clip_users = Vec::new();
        let mut originals = Vec::new();
        let mut saved = Vec::new();
        let mut created = Vec::new();
        let mut names = Vec::new();
        let mut offsets = Vec::new();
        let mut clip_sessions = Vec::new();
        for (index, clip) in clips.iter().enumerate() {
            let source = sources.clip(clip.variant).ok_or(AppError::InternalError)?;
            let f = &fragments[clip.fragment];
            clip_ids.push(clip.clip_id.clone());
            lengths.push(source.duration_ms as f32 / 1000.0);
            sizes.push(source.bytes as i64);
            clip_channels.push(f.channel_id);
            clip_users.push(clip.user_id);
            originals.push(stems[clip.fragment].clone());
            saved.push(clip_saved_name(clip));
            created.push(clip.created_ms);
            names.push(format!("load test clip {}", index + 1));
            offsets.push(clip.offset_s);
            clip_sessions.push(f.session_id);
        }
        sqlx::query(
            "INSERT INTO clips
                (clip_id, length, size, channel_id, guild_id, user_id, original_file_name,
                 saved_file_name, created_at, name, start_time, recording_session_id)
             SELECT clip_id, length, size, channel_id, $1, user_id, original, saved,
                    to_timestamp(created_ms / 1000.0), name, start_time, session_id
               FROM UNNEST($2::text[], $3::real[], $4::bigint[], $5::bigint[], $6::bigint[],
                           $7::text[], $8::text[], $9::bigint[], $10::text[], $11::real[],
                           $12::bigint[])
                 AS c(clip_id, length, size, channel_id, user_id, original, saved,
                      created_ms, name, start_time, session_id)",
        )
        .bind(guild_id)
        .bind(&clip_ids)
        .bind(&lengths)
        .bind(&sizes)
        .bind(&clip_channels)
        .bind(&clip_users)
        .bind(&originals)
        .bind(&saved)
        .bind(&created)
        .bind(&names)
        .bind(&offsets)
        .bind(&clip_sessions)
        .execute(&mut **tx)
        .await?;
    }

    if !stamps.is_empty() {
        let f = |stamp: &Stamp| &fragments[stamp.fragment];
        sqlx::query(
            "INSERT INTO stamps
                (guild_id, channel_id, target_user_id, stamper_user_id, stamp_ts, offset_ms,
                 audio_file_id, recording_session_id)
             SELECT $1, channel_id, target, stamper, stamp_ts, 0, audio_file_id, session_id
               FROM UNNEST($2::bigint[], $3::bigint[], $4::bigint[], $5::bigint[],
                           $6::bigint[], $7::bigint[])
                 AS s(channel_id, target, stamper, stamp_ts, audio_file_id, session_id)",
        )
        .bind(guild_id)
        .bind(stamps.iter().map(|s| f(s).channel_id).collect::<Vec<_>>())
        .bind(stamps.iter().map(|s| f(s).user_id).collect::<Vec<_>>())
        .bind(stamps.iter().map(|s| s.stamper).collect::<Vec<_>>())
        .bind(stamps.iter().map(|s| s.at_ms).collect::<Vec<_>>())
        .bind(
            stamps
                .iter()
                .map(|s| f(s).audio_file_id)
                .collect::<Vec<_>>(),
        )
        .bind(stamps.iter().map(|s| f(s).session_id).collect::<Vec<_>>())
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}
