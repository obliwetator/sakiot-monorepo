//! run_systemctl() from ops/lib/common.sh: either the sudo-gated wrapper
//! (unit names validated by root-owned ops/systemctl-wrapper) or plain
//! systemctl for test/dev environments.

use anyhow::Result;

use crate::runner::{Cmd, CommandRunner};

pub const WRAPPER_PATH: &str = "/usr/local/lib/sakiot-deploy/systemctl-wrapper";

pub struct Systemctl<'a> {
    runner: &'a dyn CommandRunner,
    use_sudo: bool,
}

impl<'a> Systemctl<'a> {
    pub fn new(runner: &'a dyn CommandRunner, use_sudo: bool) -> Systemctl<'a> {
        Systemctl { runner, use_sudo }
    }

    fn cmd(&self, args: &[&str]) -> Cmd {
        if self.use_sudo {
            Cmd::new("sudo")
                .arg("-n")
                .arg(WRAPPER_PATH)
                .args(args.iter().copied())
        } else {
            Cmd::new("systemctl").args(args.iter().copied())
        }
    }

    /// Fails the deploy on error (bash `run_systemctl ...` under set -e).
    pub fn run(&self, args: &[&str]) -> Result<()> {
        self.runner.run(&self.cmd(args))
    }

    /// Best-effort / condition form (`run_systemctl ... || true`, `if run_systemctl`).
    pub fn run_ok(&self, args: &[&str]) -> bool {
        self.runner.run_ok(&self.cmd(args))
    }

    /// Best-effort stop with SIGKILL escalation. Bot units set
    /// TimeoutStopSec=infinity, so a bot that ignores SIGTERM would block a
    /// plain `systemctl stop` (and deploy recovery with it) forever.
    pub fn stop_bot_bounded(&self, unit: &str, timeout: std::time::Duration) {
        match self
            .runner
            .run_ok_timeout(&self.cmd(&["stop", unit]), timeout)
        {
            Some(_) => {}
            None => {
                crate::log(format!(
                    "{unit} did not stop within {}s; escalating to SIGKILL",
                    timeout.as_secs()
                ));
                let kill = if self.use_sudo {
                    self.cmd(&["kill-bot", unit])
                } else {
                    Cmd::new("systemctl").args(["kill", "-s", "SIGKILL", unit])
                };
                let _ = self.runner.run_ok(&kill);
            }
        }
    }
}
