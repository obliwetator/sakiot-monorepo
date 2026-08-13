//! Dependency probes. The CLI reports every missing tool in one actionable
//! error and never installs anything on the developer's behalf.

use anyhow::{Result, bail};

use crate::runner::{Cmd, CommandRunner};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencySet {
    Database,
    Up,
    Fixtures,
}

pub fn check<R: CommandRunner + ?Sized>(
    runner: &R,
    set: DependencySet,
    ssh_needed: bool,
) -> Result<()> {
    let mut missing = Vec::new();
    let mut check = |name: &str, install: &str| {
        if !runner.command_exists(name) {
            missing.push(format!("{name} — {install}"));
        }
    };

    match set {
        DependencySet::Database => check("docker", "install Docker Engine with the Compose plugin"),
        DependencySet::Up => {
            check("docker", "install Docker Engine with the Compose plugin");
            check("cargo", "install Rust from rustup");
            check("cargo-watch", "cargo install cargo-watch --locked");
            check("bun", "install Bun from https://bun.sh");
            check("ffmpeg", "install FFmpeg with the rubberband filter");
            check("ffprobe", "install FFmpeg (ffprobe is normally included)");
            check(
                "audiowaveform",
                "install audiowaveform from the Chris Needham package",
            );
        }
        DependencySet::Fixtures => {
            check("rsync", "install rsync");
            if ssh_needed {
                check("ssh", "install an OpenSSH client");
            }
        }
    }

    if matches!(set, DependencySet::Database | DependencySet::Up)
        && runner.command_exists("docker")
        && !runner.run_ok(&Cmd::new("docker").args(["compose", "version"]))
    {
        missing.push("docker compose plugin — enable/install Docker Compose v2".into());
    }

    if set == DependencySet::Up && runner.command_exists("ffmpeg") {
        let filters = Cmd::new("ffmpeg").args(["-hide_banner", "-filters"]);
        match runner.run_capture(&filters) {
            Ok(output) if output.lines().any(|line| line.contains("rubberband")) => {}
            Ok(_) => missing.push(
                "ffmpeg rubberband filter — install an FFmpeg build with rubberband support".into(),
            ),
            Err(_) => missing.push(
                "ffmpeg probe — verify the installed FFmpeg binary can list its filters".into(),
            ),
        }
    }

    if missing.is_empty() {
        return Ok(());
    }
    let details = missing
        .into_iter()
        .map(|item| format!("  - {item}"))
        .collect::<Vec<_>>()
        .join("\n");
    bail!("missing development dependencies:\n{details}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::{Cmd, CommandRunner};

    struct Missing;

    impl CommandRunner for Missing {
        fn run(&self, _command: &Cmd) -> Result<()> {
            Ok(())
        }

        fn run_ok(&self, _command: &Cmd) -> bool {
            true
        }

        fn run_capture(&self, _command: &Cmd) -> Result<String> {
            Ok(String::new())
        }

        fn run_input_capture(&self, _command: &Cmd, _input: &[u8]) -> Result<String> {
            Ok(String::new())
        }

        fn command_exists(&self, _name: &str) -> bool {
            false
        }
    }

    #[test]
    fn errors_aggregate_missing_tools() {
        let error = check(&Missing, DependencySet::Up, true)
            .unwrap_err()
            .to_string();
        assert!(error.contains("docker"));
        assert!(error.contains("bun"));
        assert!(!error.contains("rsync"));
    }
}
