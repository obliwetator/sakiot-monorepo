//! Explicit subprocess boundaries with a scripted implementation for tests.

use std::io::Write;
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
    pub env_remove: Vec<String>,
    pub input: Option<Vec<u8>>,
}

impl Cmd {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
            env_remove: Vec::new(),
            input: None,
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    pub fn env_remove(mut self, key: impl Into<String>) -> Self {
        self.env_remove.push(key.into());
        self
    }

    pub fn input(mut self, input: impl Into<Vec<u8>>) -> Self {
        self.input = Some(input.into());
        self
    }

    pub fn rendered(&self) -> String {
        let mut parts = vec![self.program.clone()];
        parts.extend(self.args.iter().map(|arg| quote_for_log(arg)));
        parts.join(" ")
    }

    pub fn without_input(&self) -> Self {
        let mut command = self.clone();
        command.input = None;
        command
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.args);
        if let Some(cwd) = &self.cwd {
            command.current_dir(cwd);
        }
        for key in &self.env_remove {
            command.env_remove(key);
        }
        for (key, value) in &self.env {
            command.env(key, value);
        }
        command
    }
}

pub trait CommandRunner: Send + Sync {
    /// Run with inherited stdio and fail on a non-zero status.
    fn run(&self, command: &Cmd) -> Result<()>;

    /// Run while suppressing output and return whether it succeeded.
    fn run_ok(&self, command: &Cmd) -> bool;

    /// Run while capturing stdout and inheriting stderr.
    fn run_capture(&self, command: &Cmd) -> Result<String>;

    /// Run with input and capture stdout. This is used for remote `psql` so
    /// SQL never has to be interpolated into a shell argument.
    fn run_input_capture(&self, command: &Cmd, input: &[u8]) -> Result<String>;

    /// Check an executable without invoking a shell.
    fn command_exists(&self, name: &str) -> bool;
}

pub struct RealRunner;

impl CommandRunner for RealRunner {
    fn run(&self, command: &Cmd) -> Result<()> {
        let mut child = spawn_std(command, true)?;
        if let Some(input) = &command.input
            && let Some(mut stdin) = child.stdin.take()
        {
            stdin.write_all(input)?;
        }
        let status = child
            .wait()
            .with_context(|| format!("failed waiting for {}", command.program))?;
        if !status.success() {
            bail!("command failed ({status}): {}", command.rendered())
        }
        Ok(())
    }

    fn run_ok(&self, command: &Cmd) -> bool {
        let mut command = command.without_input().command();
        command.stdout(Stdio::null()).stderr(Stdio::null());
        command.status().is_ok_and(|status| status.success())
    }

    fn run_capture(&self, command: &Cmd) -> Result<String> {
        let input = command.input.as_deref().unwrap_or_default();
        self.run_input_capture(command, input)
    }

    fn run_input_capture(&self, command: &Cmd, input: &[u8]) -> Result<String> {
        let mut child = command.command();
        child.stdout(Stdio::piped()).stderr(Stdio::inherit());
        child.stdin(Stdio::piped());
        let mut child = child
            .spawn()
            .with_context(|| format!("failed to run {}", command.rendered()))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(input)?;
        }
        let output = child
            .wait_with_output()
            .with_context(|| format!("failed waiting for {}", command.rendered()))?;
        if !output.status.success() {
            bail!("command failed ({}): {}", output.status, command.rendered())
        }
        String::from_utf8(output.stdout)
            .with_context(|| format!("non-UTF-8 output from {}", command.rendered()))
    }

    fn command_exists(&self, name: &str) -> bool {
        if name.contains('/') {
            return std::path::Path::new(name).is_file();
        }
        std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).any(|dir| dir.join(name).is_file()))
            .unwrap_or(false)
    }
}

fn spawn_std(command: &Cmd, inherit_output: bool) -> Result<std::process::Child> {
    let mut command_process = command.command();
    if command.input.is_some() {
        command_process.stdin(Stdio::piped());
    }
    if !inherit_output {
        command_process.stdout(Stdio::null()).stderr(Stdio::null());
    }
    command_process
        .spawn()
        .with_context(|| format!("failed to run {}", command.rendered()))
}

pub fn to_tokio_command(command: &Cmd) -> tokio::process::Command {
    let mut process = tokio::process::Command::new(&command.program);
    process.args(&command.args);
    if let Some(cwd) = &command.cwd {
        process.current_dir(cwd);
    }
    for key in &command.env_remove {
        process.env_remove(key);
    }
    for (key, value) in &command.env {
        process.env(key, value);
    }
    process
}

fn quote_for_log(value: &str) -> String {
    if value.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'-' | b':')
    }) {
        value.to_string()
    } else {
        format!("'{value}'")
    }
}

#[derive(Debug, Clone)]
pub struct ScriptEntry {
    pub expect: String,
    pub stdout: String,
    pub success: bool,
}

impl ScriptEntry {
    pub fn ok(expect: impl Into<String>) -> Self {
        Self {
            expect: expect.into(),
            stdout: String::new(),
            success: true,
        }
    }

    pub fn ok_with(expect: impl Into<String>, stdout: impl Into<String>) -> Self {
        Self {
            expect: expect.into(),
            stdout: stdout.into(),
            success: true,
        }
    }

    pub fn fail(expect: impl Into<String>) -> Self {
        Self {
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

/// Exact-argv runner used by orchestration tests. It intentionally does not
/// inspect PATH and therefore tests can focus on lifecycle ordering.
#[derive(Default)]
pub struct ScriptedRunner {
    state: Mutex<ScriptState>,
}

impl ScriptedRunner {
    pub fn new(script: Vec<ScriptEntry>) -> Self {
        Self {
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

    pub fn assert_exhausted(&self) -> Result<()> {
        let state = self.state.lock().map_err(|_| anyhow::anyhow!("poisoned"))?;
        if state.cursor != state.script.len() {
            bail!(
                "unconsumed scripted command: {}",
                state.script[state.cursor].expect
            )
        }
        Ok(())
    }

    fn next(&self, command: &Cmd) -> Result<(String, bool)> {
        let mut state = self.state.lock().map_err(|_| anyhow::anyhow!("poisoned"))?;
        let rendered = command.rendered();
        state.log.push(rendered.clone());
        let index = state.cursor;
        let (expected, stdout, success) = state
            .script
            .get(index)
            .map(|entry| (entry.expect.clone(), entry.stdout.clone(), entry.success))
            .ok_or_else(|| anyhow::anyhow!("unexpected command: {rendered}"))?;
        if expected != rendered {
            bail!(
                "command mismatch at script index {index}: expected {}, got {rendered}",
                expected
            )
        }
        state.cursor += 1;
        Ok((stdout, success))
    }
}

impl CommandRunner for ScriptedRunner {
    fn run(&self, command: &Cmd) -> Result<()> {
        let (_, success) = self.next(command)?;
        if success {
            Ok(())
        } else {
            bail!("scripted command failed: {}", command.rendered())
        }
    }

    fn run_ok(&self, command: &Cmd) -> bool {
        self.next(command)
            .map(|(_, success)| success)
            .unwrap_or(false)
    }

    fn run_capture(&self, command: &Cmd) -> Result<String> {
        let (stdout, success) = self.next(command)?;
        if success {
            Ok(stdout)
        } else {
            bail!("scripted command failed: {}", command.rendered())
        }
    }

    fn run_input_capture(&self, command: &Cmd, _input: &[u8]) -> Result<String> {
        self.run_capture(command)
    }

    fn command_exists(&self, _name: &str) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripted_runner_enforces_order_and_records_argv() {
        let runner = ScriptedRunner::new(vec![
            ScriptEntry::ok("docker compose version"),
            ScriptEntry::ok_with("psql -At", "ok\n"),
        ]);
        runner
            .run(&Cmd::new("docker").args(["compose", "version"]))
            .unwrap();
        assert_eq!(
            runner.run_capture(&Cmd::new("psql").arg("-At")).unwrap(),
            "ok\n"
        );
        runner.assert_exhausted().unwrap();
        assert_eq!(runner.log(), vec!["docker compose version", "psql -At"]);
    }
}
