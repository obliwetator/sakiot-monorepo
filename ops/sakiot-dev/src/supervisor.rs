//! Coordinated backend/frontend supervision.

use std::process::Stdio;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};

use crate::config::Config;
use crate::runner::{Cmd, to_tokio_command};

#[async_trait]
pub trait ServiceSupervisor: Send + Sync {
    async fn supervise(&self, config: &Config) -> Result<()>;
}

pub struct RealServiceSupervisor;

#[async_trait]
impl ServiceSupervisor for RealServiceSupervisor {
    async fn supervise(&self, config: &Config) -> Result<()> {
        supervise_children(config).await
    }
}

pub async fn supervise(config: &Config) -> Result<()> {
    RealServiceSupervisor.supervise(config).await
}

async fn supervise_children(config: &Config) -> Result<()> {
    let backend = Cmd::new("cargo")
        .args([
            "watch",
            "-x",
            "run -p web_server --bin web_server --features dev-login",
        ])
        .cwd(&config.root);
    let frontend = Cmd::new("bun")
        .args(["run", "dev"])
        .cwd(&config.frontend_root);

    log(format!(
        "starting web_server on {} (cargo watch)",
        config.local_url
    ));
    log("starting frontend with bun run dev");
    let mut backend = spawn_child("backend", &backend)?;
    let mut frontend = match spawn_child("frontend", &frontend) {
        Ok(child) => child,
        Err(error) => {
            stop_child(&mut backend).await;
            return Err(error);
        }
    };
    let mut interrupt = Box::pin(tokio::signal::ctrl_c());

    let result = tokio::select! {
        signal = &mut interrupt => {
            signal.context("could not install Ctrl-C handler")?;
            log("Ctrl-C received; stopping backend and frontend");
            stop_child(&mut backend).await;
            stop_child(&mut frontend).await;
            Ok(())
        }
        status = backend.wait() => {
            let status = status.context("could not wait for cargo watch")?;
            log(format!("backend exited with {status}; stopping frontend"));
            stop_child(&mut frontend).await;
            if status.success() {
                Ok(())
            } else {
                bail!("backend child exited unsuccessfully ({status})")
            }
        }
        status = frontend.wait() => {
            let status = status.context("could not wait for bun")?;
            log(format!("frontend exited with {status}; stopping backend"));
            stop_child(&mut backend).await;
            if status.success() {
                Ok(())
            } else {
                bail!("frontend child exited unsuccessfully ({status})")
            }
        }
    };
    result
}

fn spawn_child(label: &'static str, command: &Cmd) -> Result<tokio::process::Child> {
    let mut process = to_tokio_command(command);
    process.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = process
        .spawn()
        .with_context(|| format!("could not start {label}: {}", command.rendered()))?;
    if let Some(stdout) = child.stdout.take() {
        tokio::spawn(prefix_output(label, stdout));
    }
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(prefix_output(label, stderr));
    }
    Ok(child)
}

async fn stop_child(child: &mut tokio::process::Child) {
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.start_kill();
    }
    let _ = child.wait().await;
}

async fn prefix_output<R>(label: &'static str, reader: R)
where
    R: AsyncRead + Unpin,
{
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        log(format!("[{label}] {line}"));
    }
}

#[expect(
    clippy::print_stdout,
    reason = "child output and progress are the local CLI product"
)]
fn log(message: impl AsRef<str>) {
    println!("[dev] {}", message.as_ref());
}
