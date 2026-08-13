use std::process::ExitCode;

use clap::{CommandFactory, Parser};
use clap_complete::generate;

use sakiot_dev::cli::{Cli, Command};
use sakiot_dev::config::discover_root;
use sakiot_dev::db::RealDatabaseFactory;
use sakiot_dev::orchestrator::{self, Deps};
use sakiot_dev::prompt::TerminalPrompt;
use sakiot_dev::runner::RealRunner;

#[expect(
    clippy::print_stderr,
    reason = "the binary reports fatal CLI errors on stderr"
)]
#[tokio::main]
async fn main() -> ExitCode {
    // Environment files contain JWT and local-login secrets. Keep generated
    // files private even when the caller's shell has a permissive umask.
    // SAFETY: umask changes only this process's file-creation mode.
    unsafe {
        libc::umask(0o077);
    }

    let cli = Cli::parse();
    if let Command::Completions(args) = &cli.command {
        let mut command = Cli::command();
        generate(
            args.shell,
            &mut command,
            "cargo-dev",
            &mut std::io::stdout(),
        );
        return ExitCode::SUCCESS;
    }
    let root = match std::env::current_dir()
        .and_then(|path| discover_root(path).map_err(std::io::Error::other))
    {
        Ok(root) => root,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::from(1);
        }
    };
    let runner = RealRunner;
    let databases = RealDatabaseFactory;
    let mut prompt = TerminalPrompt;
    let result = orchestrator::run(
        cli,
        &root,
        Deps {
            runner: &runner,
            databases: &databases,
            prompt: &mut prompt,
        },
    )
    .await;
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::from(1)
        }
    }
}
