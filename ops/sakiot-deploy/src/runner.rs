//! Subprocess execution behind a trait so the orchestrator can be tested
//! against a scripted runner instead of PATH-shimmed fake binaries.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cmd {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
}

impl Cmd {
    pub fn new(program: impl Into<String>) -> Cmd {
        Cmd {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Cmd {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Cmd
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Cmd {
        self.cwd = Some(dir.into());
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Cmd {
        self.env.push((key.into(), value.into()));
        self
    }

    /// Rendered argv for logs and test assertions.
    pub fn rendered(&self) -> String {
        let mut parts = vec![self.program.clone()];
        parts.extend(self.args.iter().cloned());
        parts.join(" ")
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.args);
        if let Some(cwd) = &self.cwd {
            command.current_dir(cwd);
        }
        for (key, value) in &self.env {
            command.env(key, value);
        }
        command
    }
}

pub trait CommandRunner {
    /// Run with inherited stdio; error on non-zero exit (bash `set -e`).
    fn run(&self, cmd: &Cmd) -> Result<()>;

    /// Run with output discarded; report success (bash `cmd || true` and
    /// `if cmd; then` conditions).
    fn run_ok(&self, cmd: &Cmd) -> bool;

    /// Run capturing stdout (stderr inherited); error on non-zero exit.
    fn run_capture(&self, cmd: &Cmd) -> Result<String>;

    /// Like run_ok, but gives up after `timeout`, killing the child and
    /// returning None. The default ignores the timeout so test runners keep
    /// their exact-argv scripts.
    fn run_ok_timeout(&self, cmd: &Cmd, timeout: std::time::Duration) -> Option<bool> {
        let _ = timeout;
        Some(self.run_ok(cmd))
    }
}

pub struct RealRunner;

impl CommandRunner for RealRunner {
    fn run(&self, cmd: &Cmd) -> Result<()> {
        let status = cmd
            .command()
            .status()
            .with_context(|| format!("failed to run {}", cmd.program))?;
        if !status.success() {
            bail!("command failed ({status}): {}", cmd.rendered());
        }
        Ok(())
    }

    fn run_ok(&self, cmd: &Cmd) -> bool {
        cmd.command()
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    fn run_capture(&self, cmd: &Cmd) -> Result<String> {
        let output = cmd
            .command()
            .stderr(Stdio::inherit())
            .output()
            .with_context(|| format!("failed to run {}", cmd.program))?;
        if !output.status.success() {
            bail!("command failed ({}): {}", output.status, cmd.rendered());
        }
        String::from_utf8(output.stdout)
            .with_context(|| format!("non-UTF-8 output from {}", cmd.rendered()))
    }

    fn run_ok_timeout(&self, cmd: &Cmd, timeout: std::time::Duration) -> Option<bool> {
        let Ok(mut child) = cmd
            .command()
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            return Some(false);
        };
        let deadline = std::time::Instant::now() + timeout;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return Some(status.success()),
                Ok(None) => {}
                Err(_) => return Some(false),
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
}

/// One scripted response. The runner asserts commands arrive in script order
/// with exactly the expected argv, which doubles as the golden-sequence
/// assertion in the end-to-end tests.
pub struct ScriptEntry {
    pub expect: String,
    pub stdout: String,
    pub success: bool,
}

impl ScriptEntry {
    pub fn ok(expect: impl Into<String>) -> ScriptEntry {
        ScriptEntry {
            expect: expect.into(),
            stdout: String::new(),
            success: true,
        }
    }

    pub fn ok_with(expect: impl Into<String>, stdout: impl Into<String>) -> ScriptEntry {
        ScriptEntry {
            expect: expect.into(),
            stdout: stdout.into(),
            success: true,
        }
    }

    pub fn fail(expect: impl Into<String>) -> ScriptEntry {
        ScriptEntry {
            expect: expect.into(),
            stdout: String::new(),
            success: false,
        }
    }
}

#[derive(Default)]
struct ScriptState {
    script: Vec<ScriptEntry>,
    cursor: usize,
    log: Vec<String>,
}

/// Test runner: feeds scripted responses and records every command.
#[derive(Default)]
pub struct ScriptedRunner {
    state: Mutex<ScriptState>,
}

impl ScriptedRunner {
    pub fn new(script: Vec<ScriptEntry>) -> ScriptedRunner {
        ScriptedRunner {
            state: Mutex::new(ScriptState {
                script,
                cursor: 0,
                log: Vec::new(),
            }),
        }
    }

    pub fn log(&self) -> Vec<String> {
        self.state
            .lock()
            .map(|state| state.log.clone())
            .unwrap_or_default()
    }

    /// Errors if scripted entries were never consumed.
    pub fn assert_exhausted(&self) -> Result<()> {
        let state = self.state.lock().map_err(|_| anyhow::anyhow!("poisoned"))?;
        if state.cursor != state.script.len() {
            bail!(
                "unconsumed scripted commands starting at: {}",
                state.script[state.cursor].expect
            );
        }
        Ok(())
    }

    fn next(&self, cmd: &Cmd) -> Result<(String, bool)> {
        let mut state = self.state.lock().map_err(|_| anyhow::anyhow!("poisoned"))?;
        let rendered = cmd.rendered();
        state.log.push(rendered.clone());
        let index = state.cursor;
        let Some(entry) = state.script.get(index) else {
            bail!("unexpected command (script exhausted): {rendered}");
        };
        if entry.expect != rendered {
            bail!(
                "command mismatch at script index {index}:\n  expected: {}\n  actual:   {rendered}",
                entry.expect
            );
        }
        let result = (entry.stdout.clone(), entry.success);
        state.cursor += 1;
        Ok(result)
    }
}

impl CommandRunner for ScriptedRunner {
    fn run(&self, cmd: &Cmd) -> Result<()> {
        let (_, success) = self.next(cmd)?;
        if !success {
            bail!("command failed (scripted): {}", cmd.rendered());
        }
        Ok(())
    }

    fn run_ok(&self, cmd: &Cmd) -> bool {
        self.next(cmd).map(|(_, success)| success).unwrap_or(false)
    }

    fn run_capture(&self, cmd: &Cmd) -> Result<String> {
        let (stdout, success) = self.next(cmd)?;
        if !success {
            bail!("command failed (scripted): {}", cmd.rendered());
        }
        Ok(stdout)
    }
}
