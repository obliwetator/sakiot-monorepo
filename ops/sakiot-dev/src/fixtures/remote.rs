//! Remote PostgreSQL, SSH, archive hydration, and permission fallbacks.
//!
//! The shell is only used as the explicit remote command boundary required by
//! existing deployment env files. SQL itself is sent on stdin, and every local
//! subprocess is represented as typed argv.

use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::config::{Config, RemoteConfig};
use crate::runner::{Cmd, CommandRunner};

pub struct RemoteSql<'a, R: CommandRunner + ?Sized> {
    runner: &'a R,
    ssh: String,
    psql: String,
}

impl<'a, R: CommandRunner + ?Sized> RemoteSql<'a, R> {
    pub fn configure_read(
        runner: &'a R,
        config: &Config,
        source: &RemoteConfig,
        ssh: &str,
    ) -> Result<Self> {
        let candidates = if let Some(override_command) = &config.remote_psql {
            vec![format!(
                "{} -d {}",
                override_command,
                quote_shell(&source.database)
            )]
        } else {
            vec![
                format!(
                    "set -a; . {}; set +a; psql \"$DATABASE_URL\"",
                    quote_shell(&source.env_file.display().to_string())
                ),
                format!(
                    "sudo -n -u postgres psql -d {}",
                    quote_shell(&source.database)
                ),
                format!("psql -d {}", quote_shell(&source.database)),
            ]
        };
        let psql =
            choose_candidate(runner, ssh, &candidates, READ_PROBE_SQL).with_context(|| {
                format!(
                    "no readable PostgreSQL candidate for {} ({})",
                    source.database,
                    source.source.label()
                )
            })?;
        Ok(Self {
            runner,
            ssh: ssh.to_string(),
            psql,
        })
    }

    pub fn configure_write(
        runner: &'a R,
        config: &Config,
        source: &RemoteConfig,
        ssh: &str,
    ) -> Result<Self> {
        let candidates = if let Some(override_command) = &config.staging_psql {
            vec![override_command.clone()]
        } else {
            vec![
                format!(
                    "set -a; . {}; set +a; psql \"$DATABASE_URL\"",
                    quote_shell(&config.staging_env_file.display().to_string())
                ),
                format!(
                    "sudo -n -u sakiot /bin/bash -lc {}",
                    quote_shell(&format!(
                        "set -a; . {}; set +a; psql \"$DATABASE_URL\"",
                        quote_shell(&config.staging_env_file.display().to_string())
                    ))
                ),
                format!("psql -d {}", quote_shell(&config.staging_db)),
            ]
        };
        let psql = choose_candidate(runner, ssh, &candidates, WRITE_PROBE_SQL)
            .with_context(|| format!("no writable PostgreSQL candidate for {}", source.database))?;
        Ok(Self {
            runner,
            ssh: ssh.to_string(),
            psql,
        })
    }

    pub fn copy_out(&self, query: &str) -> Result<Vec<u8>> {
        let sql = format!("COPY ({query}) TO STDOUT;");
        let output = self.run_psql(&sql)?;
        Ok(output.into_bytes())
    }

    pub fn execute(&self, sql: &[u8]) -> Result<()> {
        self.run_psql_bytes(sql).map(|_| ())
    }

    pub fn scalar(&self, query: &str) -> Result<String> {
        let output = self.run_psql(&format!("COPY ({query}) TO STDOUT;"))?;
        Ok(output.trim().to_string())
    }

    pub fn table_columns(&self, table: &str) -> Result<Vec<String>> {
        let table = table.replace('\'', "''");
        let output = self.scalar(&format!(
            "SELECT column_name FROM information_schema.columns WHERE table_schema = 'public' AND table_name = '{table}' ORDER BY ordinal_position"
        ))?;
        Ok(output
            .lines()
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect())
    }

    pub fn run_psql(&self, sql: &str) -> Result<String> {
        self.run_psql_bytes(sql.as_bytes())
    }

    fn run_psql_bytes(&self, sql: &[u8]) -> Result<String> {
        let command = remote_shell(
            Some(&self.ssh),
            &format!("{} -v ON_ERROR_STOP=1 -At", self.psql),
        );
        self.runner
            .run_input_capture(&command, sql)
            .with_context(|| format!("remote PostgreSQL command failed: {}", command.rendered()))
    }

    pub fn remote_capture(&self, command: &str) -> Result<String> {
        self.runner
            .run_capture(&remote_shell(Some(&self.ssh), command))
    }

    pub fn remote_ok(&self, command: &str) -> bool {
        self.runner.run_ok(&remote_shell(Some(&self.ssh), command))
    }
}

pub fn hydrate_remote_media<R: CommandRunner + ?Sized>(
    runner: &R,
    config: &Config,
    source: &RemoteConfig,
    ssh: &str,
    what: &str,
    flag: &str,
    ids: &str,
) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let command = if let Some(custom) = &config.remote_hydrate {
        format!("{custom} {} {}", quote_shell(flag), quote_shell(ids))
    } else {
        format!(
            "set -a; . {}; set +a; {} media restore {} {}",
            quote_shell(&source.env_file.display().to_string()),
            quote_shell(&source.binary.display().to_string()),
            quote_shell(flag),
            quote_shell(ids)
        )
    };
    let fallback = format!("sudo -n -u sakiot /bin/bash -lc {}", quote_shell(&command));
    log(format!(
        "materializing selected {} {} media",
        source.source.label(),
        what
    ));
    let first = remote_shell(Some(ssh), &command);
    if runner.run_ok(&first) {
        return Ok(());
    }
    if runner.run_ok(&remote_shell(Some(ssh), &fallback)) {
        return Ok(());
    }
    bail!(
        "could not hydrate {} archive media ({what}); grant env/data access, passwordless sudo -u sakiot, or set SAKIOT_DEV_REMOTE_HYDRATE",
        source.source.label()
    )
}

pub fn remote_missing_files<R: CommandRunner + ?Sized>(
    remote: &RemoteSql<'_, R>,
    data_dir: &Path,
    files: &[String],
) -> Result<Vec<String>> {
    let script = files
        .iter()
        .filter(|file| !file.is_empty())
        .map(|file| {
            format!(
                "test -f {} || printf '%s\\n' {}",
                quote_shell(&data_dir.join(file).display().to_string()),
                quote_shell(file)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    if script.is_empty() {
        return Ok(Vec::new());
    }
    Ok(remote
        .remote_capture(&script)?
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

pub fn copy_media<R: CommandRunner + ?Sized>(
    runner: &R,
    ssh: &str,
    source_data: &Path,
    files_from: &Path,
    workspace_media: &Path,
) -> Result<()> {
    let source = if ssh == "local" {
        format!("{}/", source_data.display())
    } else {
        format!("{}:{}/", ssh, source_data.display())
    };
    runner.run(&Cmd::new("rsync").args([
        "-a",
        "--info=stats1",
        "--ignore-missing-args",
        &format!("--files-from={}", files_from.display()),
        &source,
        &format!("{}/", workspace_media.display()),
    ]))?;
    Ok(())
}

pub fn quote_shell(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn choose_candidate<R: CommandRunner + ?Sized>(
    runner: &R,
    ssh: &str,
    candidates: &[String],
    probe: &str,
) -> Result<String> {
    for candidate in candidates {
        let command = remote_shell(Some(ssh), &format!("{candidate} -v ON_ERROR_STOP=1 -At"));
        if let Ok(output) = runner.run_input_capture(&command, probe.as_bytes())
            && output.trim() == "ok"
        {
            return Ok(candidate.clone());
        }
    }
    bail!("all configured remote PostgreSQL candidates failed their privilege probe")
}

fn remote_shell(ssh: Option<&str>, command: &str) -> Cmd {
    match ssh {
        Some(ssh) if ssh != "local" => Cmd::new("ssh").args([ssh, command]),
        _ => Cmd::new("sh").args(["-lc", command]),
    }
}

const READ_PROBE_SQL: &str = "SELECT 'ok' WHERE has_table_privilege('audio_files', 'SELECT') AND has_table_privilege('recording_sessions', 'SELECT') AND has_table_privilege('clips', 'SELECT') AND has_table_privilege('stamps', 'SELECT')";
const WRITE_PROBE_SQL: &str = "SELECT 'ok' WHERE has_table_privilege('audio_files', 'INSERT') AND has_table_privilege('recording_sessions', 'INSERT') AND has_table_privilege('clips', 'INSERT') AND has_table_privilege('stamps', 'INSERT')";

#[expect(
    clippy::print_stdout,
    reason = "fixture progress is intentionally human-readable"
)]
fn log(message: String) {
    println!("[dev] {message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_quote_handles_single_quotes() {
        assert_eq!(quote_shell("a'b"), "'a'\\''b'");
    }
}
