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

fn run(request: Request) -> Result<()> {
    let config = Config::load(request.target)?;
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
        free_port: &free_port,
        require_command: &require_command,
    };
    deploy::run(&request, &config, &deps)
}
