#![expect(
    clippy::print_stderr,
    reason = "usage and fatal errors go to stderr: `die` parity with the bash engine"
)]

use anyhow::{Context, Result};
use sakiot_deploy::admin_api::TonicAdmin;
use sakiot_deploy::clock::{self, SystemClock};
use sakiot_deploy::config::{Config, Request, UsageError};
use sakiot_deploy::deploy::{self, Deps};
use sakiot_deploy::runner::RealRunner;
use sakiot_deploy::web_api::ReqwestWebApi;

fn main() {
    // umask 027, as the first line of deploy-release.sh.
    // SAFETY: umask has no failure modes and no memory effects.
    unsafe {
        libc::umask(0o027);
    }

    let args: Vec<String> = std::env::args().skip(1).collect();

    // Local-only read-only snapshot; not part of the deploy request grammar.
    if args.first().map(String::as_str) == Some("status") {
        let (target, slot) = match args.get(1).map(String::as_str) {
            Some("production") if args.len() == 2 => {
                (sakiot_deploy::config::Target::Production, None)
            }
            Some("staging") if args.len() == 2 => (sakiot_deploy::config::Target::Staging, None),
            Some("preview") if args.len() == 2 => (sakiot_deploy::config::Target::Preview, None),
            Some("preview") if args.len() == 3 => {
                let slot = args[2].clone();
                if !sakiot_deploy::config::is_valid_slot(&slot) {
                    eprintln!("usage: sakiot-deploy status {{production|staging|preview [slot]}}");
                    std::process::exit(2);
                }
                (sakiot_deploy::config::Target::Preview, Some(slot))
            }
            _ => {
                eprintln!("usage: sakiot-deploy status {{production|staging|preview [slot]}}");
                std::process::exit(2);
            }
        };
        if let Err(error) = status(target, slot.as_deref()) {
            eprintln!("error: {error:#}");
            std::process::exit(1);
        }
        return;
    }

    let request = match Request::parse(args) {
        Ok(request) => request,
        Err(UsageError(usage)) => {
            eprintln!("{usage}");
            std::process::exit(2);
        }
    };

    if let Err(error) = run(request) {
        // `die` parity with ops/lib/common.sh.
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn status(target: sakiot_deploy::config::Target, slot: Option<&str>) -> Result<()> {
    let config = Config::load(target, slot)?;
    let admin = TonicAdmin::new()?;
    sakiot_deploy::status::run(target, &config, &admin)
}

/// ops/update-deploy-engine.sh stamps the ops/sakiot-deploy tree OID it
/// built from next to the installed binary (<install root>/engine-src-tree).
/// A missing or unreadable stamp disables the deploy-time drift warning.
fn engine_src_tree() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let stamp = exe.parent()?.parent()?.join("engine-src-tree");
    let content = std::fs::read_to_string(stamp).ok()?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

fn run(request: Request) -> Result<()> {
    let config = Config::load(request.target, request.slot.as_deref())?;
    let runner = RealRunner;
    let admin = TonicAdmin::new()?;
    let web = ReqwestWebApi::new()?;
    let clock = SystemClock;
    let free_port = || -> Result<u16> {
        // Replaces the inline python3 socket bind in deploy-release.sh.
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .context("failed to pick a free gRPC port")?;
        Ok(listener.local_addr()?.port())
    };
    let require_command = |name: &str| deploy::require_command(name);
    let deps = Deps {
        runner: &runner,
        admin: &admin,
        web: &web,
        clock: &clock,
        hostname: clock::hostname(),
        engine_src_tree: engine_src_tree(),
        free_port: &free_port,
        require_command: &require_command,
    };
    deploy::run(&request, &config, &deps)
}
