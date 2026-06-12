//! Golden command-sequence tests for the deploy orchestrator. Each scenario
//! scripts the exact subprocess sequence the engine must produce (the
//! ScriptedRunner fails on any divergence) and checks state files, the
//! manifest, and the recovery flows the bash engine never had tests for.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;
use sakiot_deploy::admin_api::AdminApi;
use sakiot_deploy::clock::Clock;
use sakiot_deploy::config::{Config, Request};
use sakiot_deploy::deploy::{self, Deps};
use sakiot_deploy::runner::{ScriptEntry, ScriptedRunner};
use sakiot_deploy::web_api::WebApi;
use time::OffsetDateTime;

const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
// 2026-01-15T12:00:00Z -> compact stamp 20260115T120000Z.
const NOW_UNIX: i64 = 1_768_478_400;
const STAMP: &str = "20260115T120000Z";
const PORT: u16 = 40123;

#[derive(Default)]
struct Events(Mutex<Vec<String>>);

impl Events {
    fn push(&self, event: String) {
        if let Ok(mut events) = self.0.lock() {
            events.push(event);
        }
    }

    fn all(&self) -> Vec<String> {
        self.0
            .lock()
            .map(|events| events.clone())
            .unwrap_or_default()
    }
}

struct MockAdmin<'a> {
    events: &'a Events,
    /// drain_status_ok returns false this many times, then true.
    status_failures: usize,
    status_calls: Mutex<usize>,
}

impl<'a> MockAdmin<'a> {
    fn ready(events: &'a Events) -> MockAdmin<'a> {
        MockAdmin {
            events,
            status_failures: 0,
            status_calls: Mutex::new(0),
        }
    }

    fn never_ready(events: &'a Events) -> MockAdmin<'a> {
        MockAdmin {
            events,
            status_failures: usize::MAX,
            status_calls: Mutex::new(0),
        }
    }
}

impl AdminApi for MockAdmin<'_> {
    fn start_drain(&self, address: &str, reason: &str) -> Result<()> {
        self.events
            .push(format!("admin start_drain {address} '{reason}'"));
        Ok(())
    }

    fn cancel_drain(&self, address: &str, reason: &str) -> Result<()> {
        self.events
            .push(format!("admin cancel_drain {address} '{reason}'"));
        Ok(())
    }

    fn shutdown_when_empty(&self, address: &str, reason: &str) -> Result<()> {
        self.events
            .push(format!("admin shutdown_when_empty {address} '{reason}'"));
        Ok(())
    }

    fn drain_status_ok(&self, address: &str) -> bool {
        self.events.push(format!("admin drain_status {address}"));
        let mut calls = match self.status_calls.lock() {
            Ok(calls) => calls,
            Err(_) => return false,
        };
        *calls += 1;
        *calls > self.status_failures
    }

    fn drain_status(&self, address: &str) -> Result<sakiot_proto::fbi_agent::DrainStatus> {
        anyhow::bail!("drain_status not used in deploy flows ({address})")
    }
}

struct MockWeb<'a> {
    events: &'a Events,
    healthy: bool,
}

impl WebApi for MockWeb<'_> {
    fn health_ready(&self, url: &str, release_id: &str) -> bool {
        self.events.push(format!("web health {url} {release_id}"));
        self.healthy
    }

    fn publish_registry(
        &self,
        url: &str,
        _secret: &str,
        active: &str,
        draining: &[String],
    ) -> Result<()> {
        self.events.push(format!(
            "web publish {url} active={active} draining={draining:?}"
        ));
        Ok(())
    }
}

struct MockClock {
    sleeps: Mutex<u32>,
}

impl MockClock {
    fn new() -> MockClock {
        MockClock {
            sleeps: Mutex::new(0),
        }
    }

    fn sleep_count(&self) -> u32 {
        self.sleeps.lock().map(|sleeps| *sleeps).unwrap_or(0)
    }
}

impl Clock for MockClock {
    fn now_utc(&self) -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(NOW_UNIX).unwrap_or(OffsetDateTime::UNIX_EPOCH)
    }

    fn sleep(&self, _duration: Duration) {
        if let Ok(mut sleeps) = self.sleeps.lock() {
            *sleeps += 1;
        }
    }
}

struct World {
    _root: tempfile::TempDir,
    config: Config,
}

impl World {
    fn new() -> World {
        let root = tempfile::tempdir().unwrap();
        let base = root.path().canonicalize().unwrap();
        let config = Config {
            env_file: base.join("deploy.env"),
            web_unit: "sakiot-web.service".into(),
            bot_unit_prefix: "sakiot-fbi-agent@".into(),
            repository_url: "https://example.invalid/sakiot.git".into(),
            data_dir: base.join("data"),
            state_dir: base.join("state"),
            release_root: base.join("releases"),
            current_root: base.join("current"),
            cache_dir: base.join("cache"),
            frontend_root: base.join("www"),
            web_health_url: "http://127.0.0.1:1/healthz".into(),
            web_registry_url: "http://127.0.0.1:1/registry".into(),
            legacy_bot_unit: String::new(),
            legacy_bot_grpc: String::new(),
            legacy_web_enabled: false,
            systemctl_use_sudo: false,
            skip_db_backup: true,
            rollback_force_rebuild: false,
            keep_releases: "5".into(),
            registry_secret: String::new(),
            database_url: Some("postgres://localhost/sakiot_rouvas".into()),
            test_database_url: "postgres://localhost/sakiot_test".into(),
        };
        // The repository cache must look like an existing clone, and the fake
        // cargo target dir holds the "built" binaries that get installed.
        fs::create_dir_all(config.source_repo().join(".git")).unwrap();
        fs::create_dir_all(config.cache_dir.join("cargo-target/release")).unwrap();
        fs::write(
            config.cache_dir.join("cargo-target/release/fbi_agent"),
            "bot-bin",
        )
        .unwrap();
        fs::write(
            config.cache_dir.join("cargo-target/release/web_server"),
            "web-bin",
        )
        .unwrap();
        World {
            _root: root,
            config,
        }
    }

    fn repo(&self) -> String {
        self.config.source_repo().display().to_string()
    }

    fn worktree(&self, release_id: &str) -> PathBuf {
        self.config.worktree_root().join(release_id)
    }

    /// The worktree contents the engine reads from the checked-out SHA.
    fn fake_worktree(&self, release_id: &str) {
        let worktree = self.worktree(release_id);
        fs::create_dir_all(worktree.join("sakiot-db/migrations")).unwrap();
        fs::write(
            worktree.join("sakiot-db/migrations/0001_init.sql"),
            "create table x;",
        )
        .unwrap();
    }

    fn write_state(&self, name: &str, value: &str) {
        fs::create_dir_all(&self.config.state_dir).unwrap();
        fs::write(self.config.state_dir.join(name), format!("{value}\n")).unwrap();
    }

    fn state(&self, name: &str) -> Option<String> {
        fs::read_to_string(self.config.state_dir.join(name))
            .ok()
            .map(|content| content.trim_end_matches('\n').to_string())
    }

    fn run(
        &self,
        args: &[&str],
        runner: &ScriptedRunner,
        admin: &dyn AdminApi,
        web: &dyn WebApi,
        clock: &dyn Clock,
    ) -> Result<()> {
        let request = Request::parse(args.iter().map(|s| s.to_string()).collect())
            .map_err(|usage| anyhow::anyhow!(usage.0))?;
        let free_port = || Ok(PORT);
        let require_command = |_: &str| Ok(());
        let deps = Deps {
            runner,
            admin,
            web,
            clock,
            hostname: "testhost".into(),
            free_port: &free_port,
            require_command: &require_command,
        };
        deploy::run(&request, &self.config, &deps)
    }
}

fn git(repo: &str, rest: &str) -> String {
    format!("git -C {repo} {rest}")
}

#[test]
fn stage_happy_path_web_only() {
    let world = World::new();
    let repo = world.repo();
    let release_id = format!("staging-bbbbbbbbbbbb-{STAMP}");
    let worktree = world.worktree(&release_id).display().to_string();
    world.fake_worktree(&release_id);
    world.write_state("current.sha", SHA_A);
    world.write_state("current.tag", "main");

    let runner = ScriptedRunner::new(vec![
        ScriptEntry::ok(git(
            &repo,
            "remote set-url origin https://example.invalid/sakiot.git",
        )),
        ScriptEntry::ok(git(
            &repo,
            "fetch --prune origin +refs/heads/*:refs/remotes/origin/*",
        )),
        ScriptEntry::ok(git(&repo, &format!("cat-file -e {SHA_B}^{{commit}}"))),
        ScriptEntry::ok_with(
            git(&repo, &format!("diff --name-only {SHA_A} {SHA_B}")),
            "web_server/src/main.rs\n",
        ),
        ScriptEntry::fail(git(&repo, &format!("worktree remove --force {worktree}"))),
        ScriptEntry::ok(git(
            &repo,
            &format!("worktree add --detach {worktree} {SHA_B}"),
        )),
        ScriptEntry::ok("cargo test --workspace --locked"),
        ScriptEntry::ok("cargo build --release --locked --package web_server"),
        ScriptEntry::ok("systemctl restart sakiot-web.service"),
        ScriptEntry::ok("systemctl enable-web sakiot-web.service"),
        ScriptEntry::ok(git(&repo, &format!("worktree remove --force {worktree}"))),
    ]);
    let events = Events::default();
    let admin = MockAdmin::ready(&events);
    let web = MockWeb {
        events: &events,
        healthy: true,
    };
    let clock = MockClock::new();

    world
        .run(&["stage", SHA_B], &runner, &admin, &web, &clock)
        .unwrap();
    runner.assert_exhausted().unwrap();

    assert_eq!(world.state("current.sha").as_deref(), Some(SHA_B));
    assert_eq!(world.state("current.tag").as_deref(), Some("main"));
    let artifact = world.config.release_root.join(&release_id);
    assert_eq!(
        world.state("current.manifest").as_deref(),
        Some(
            artifact
                .join("manifest.json")
                .display()
                .to_string()
                .as_str()
        )
    );

    let manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(artifact.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["mode"], "stage");
    assert_eq!(manifest["target"], "staging");
    assert_eq!(manifest["components"], serde_json::json!(["web"]));
    assert_eq!(
        manifest["changed_paths"],
        serde_json::json!(["web_server/src/main.rs"])
    );
    assert_eq!(manifest["database"]["migrations_ran"], false);
    assert_eq!(manifest["database"]["migration_head"], "0001_init.sql");
    assert_eq!(manifest["previous_sha"], SHA_A);
    assert_eq!(manifest["deployed_at"], "2026-01-15T12:00:00Z");

    // Web binary installed from the cargo target dir, service env rendered,
    // symlink swapped to this release.
    assert_eq!(
        fs::read_to_string(artifact.join("web/web_server")).unwrap(),
        "web-bin"
    );
    let service_env = fs::read_to_string(artifact.join("web/service.env")).unwrap();
    assert!(service_env.contains(&format!("RELEASE_ID={release_id}")));
    assert!(service_env.contains("GRPC_ADDRESS=http://127.0.0.1:50052"));
    assert_eq!(
        fs::read_link(world.config.current_root.join("web")).unwrap(),
        artifact.join("web")
    );
    assert_eq!(
        events.all(),
        vec![
            format!("web health http://127.0.0.1:1/healthz {release_id}"),
            // Post-restart republish; no handoff, so active comes from state.
            "web publish http://127.0.0.1:1/registry active=127.0.0.1:50052 draining=[]"
                .to_string(),
        ]
    );
    assert_eq!(clock.sleep_count(), 0);
}

#[test]
fn release_happy_path_first_deploy_all_components() {
    let world = World::new();
    let repo = world.repo();
    let release_id = format!("v1.0.0-bbbbbbbbbbbb-{STAMP}");
    let worktree_path = world.worktree(&release_id);
    let worktree = worktree_path.display().to_string();
    world.fake_worktree(&release_id);
    let migrations = worktree_path
        .join("sakiot-db/migrations")
        .display()
        .to_string();
    let new_unit = format!("sakiot-fbi-agent@{release_id}.service");

    let runner = ScriptedRunner::new(vec![
        ScriptEntry::ok(git(
            &repo,
            "remote set-url origin https://example.invalid/sakiot.git",
        )),
        ScriptEntry::ok(git(
            &repo,
            "fetch --prune origin +refs/heads/*:refs/remotes/origin/* +refs/tags/v1.0.0:refs/tags/v1.0.0",
        )),
        ScriptEntry::ok_with(
            git(&repo, "rev-list -n 1 refs/tags/v1.0.0"),
            format!("{SHA_B}\n"),
        ),
        ScriptEntry::ok(git(&repo, &format!("cat-file -e {SHA_B}^{{commit}}"))),
        ScriptEntry::fail(git(&repo, &format!("worktree remove --force {worktree}"))),
        ScriptEntry::ok(git(
            &repo,
            &format!("worktree add --detach {worktree} {SHA_B}"),
        )),
        ScriptEntry::ok("cargo test --workspace --locked"),
        ScriptEntry::ok("cargo build --release --locked --package fbi_agent"),
        ScriptEntry::ok("cargo build --release --locked --package web_server"),
        ScriptEntry::ok("bun install --frozen-lockfile"),
        ScriptEntry::ok("bun run test"),
        ScriptEntry::ok("bun run build"),
        ScriptEntry::ok(format!(
            "cp -a {worktree}/sakiot_stage/dist {}/frontend/dist",
            world.config.release_root.join(&release_id).display()
        )),
        ScriptEntry::ok(format!("sqlx migrate info --source {migrations}")),
        ScriptEntry::ok(format!("sqlx migrate run --source {migrations}")),
        ScriptEntry::ok(format!("systemctl start {new_unit}")),
        ScriptEntry::ok(format!("systemctl enable {new_unit}")),
        ScriptEntry::ok("systemctl restart sakiot-web.service"),
        ScriptEntry::ok("systemctl enable-web sakiot-web.service"),
        ScriptEntry::ok(format!("{worktree}/sakiot_stage/scripts/deploy.sh")),
        ScriptEntry::ok(git(&repo, &format!("worktree remove --force {worktree}"))),
    ]);
    let events = Events::default();
    let admin = MockAdmin::ready(&events);
    let web = MockWeb {
        events: &events,
        healthy: true,
    };
    let clock = MockClock::new();

    world
        .run(&["release", "v1.0.0", SHA_B], &runner, &admin, &web, &clock)
        .unwrap();
    runner.assert_exhausted().unwrap();

    let artifact = world.config.release_root.join(&release_id);
    let manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(artifact.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["mode"], "release");
    assert_eq!(manifest["target"], "production");
    assert_eq!(
        manifest["components"],
        serde_json::json!(["database", "bot", "web", "frontend"])
    );
    assert_eq!(manifest["changed_paths"], serde_json::json!([]));
    assert_eq!(manifest["database"]["migrations_ran"], true);
    assert_eq!(manifest["previous_sha"], "");

    // Tag record written; bot state points at the new instance.
    assert_eq!(
        fs::read_to_string(world.config.state_dir.join("tags/v1.0.0")).unwrap(),
        format!("{SHA_B}\n")
    );
    assert_eq!(
        world.state("current-bot.unit").as_deref(),
        Some(new_unit.as_str())
    );
    assert_eq!(
        world.state("current-bot.grpc").as_deref(),
        Some(format!("127.0.0.1:{PORT}").as_str())
    );

    let bot_env = fs::read_to_string(artifact.join("fbi-agent/service.env")).unwrap();
    assert!(bot_env.contains(&format!("BOT_INSTANCE_ID=testhost-{release_id}")));
    assert!(bot_env.contains(&format!("GRPC_ADDR=127.0.0.1:{PORT}")));

    // Web points at the freshly started bot.
    let web_env = fs::read_to_string(artifact.join("web/service.env")).unwrap();
    assert!(web_env.contains(&format!("GRPC_ADDRESS=http://127.0.0.1:{PORT}")));

    assert_eq!(
        events.all(),
        vec![
            format!("admin drain_status 127.0.0.1:{PORT}"),
            format!("web publish http://127.0.0.1:1/registry active=127.0.0.1:{PORT} draining=[]"),
            format!("web health http://127.0.0.1:1/healthz {release_id}"),
            // The web restart wiped the in-memory registry; republished.
            format!("web publish http://127.0.0.1:1/registry active=127.0.0.1:{PORT} draining=[]"),
        ]
    );
}

#[test]
fn rollback_reuses_artifacts_without_building() {
    let world = World::new();
    let repo = world.repo();
    let release_id = format!("v1.0.0-aaaaaaaaaaaa-rollback-{STAMP}");
    let worktree = world.worktree(&release_id).display().to_string();
    world.fake_worktree(&release_id);

    // Prior release of SHA_A with all artifacts; current state is SHA_B with
    // an active bot from that release.
    let prior = world
        .config
        .release_root
        .join("v1.0.0-aaaaaaaaaaaa-20260101T120000Z");
    fs::create_dir_all(prior.join("fbi-agent")).unwrap();
    fs::write(prior.join("fbi-agent/fbi_agent"), "old-bot-bin").unwrap();
    fs::create_dir_all(prior.join("web")).unwrap();
    fs::write(prior.join("web/web_server"), "old-web-bin").unwrap();
    fs::create_dir_all(prior.join("frontend/dist")).unwrap();
    fs::write(prior.join("frontend/dist/index.html"), "x").unwrap();
    fs::write(
        prior.join("manifest.json"),
        format!("{{\"sha\":\"{SHA_A}\"}}\n"),
    )
    .unwrap();

    let old_unit = "sakiot-fbi-agent@v1.0.1-bbbbbbbbbbbb-20260110T120000Z.service";
    world.write_state("current.sha", SHA_B);
    world.write_state("current.tag", "v1.0.1");
    world.write_state("current-bot.unit", old_unit);
    world.write_state("current-bot.grpc", "127.0.0.1:40000");
    fs::create_dir_all(world.config.state_dir.join("tags")).unwrap();
    fs::write(
        world.config.state_dir.join("tags/v1.0.0"),
        format!("{SHA_A}\n"),
    )
    .unwrap();

    let new_unit = format!("sakiot-fbi-agent@{release_id}.service");
    let prior_unit = "sakiot-fbi-agent@v1.0.0-aaaaaaaaaaaa-20260101T120000Z.service";
    let artifact = world.config.release_root.join(&release_id);

    let runner = ScriptedRunner::new(vec![
        ScriptEntry::ok(git(
            &repo,
            "remote set-url origin https://example.invalid/sakiot.git",
        )),
        ScriptEntry::ok(git(
            &repo,
            "fetch --prune origin +refs/heads/*:refs/remotes/origin/* +refs/tags/v1.0.0:refs/tags/v1.0.0",
        )),
        ScriptEntry::ok_with(
            git(&repo, "rev-list -n 1 refs/tags/v1.0.0"),
            format!("{SHA_A}\n"),
        ),
        ScriptEntry::ok(git(&repo, &format!("cat-file -e {SHA_A}^{{commit}}"))),
        ScriptEntry::ok_with(
            git(
                &repo,
                &format!("diff --name-only {SHA_A} {SHA_B} -- sakiot-db/migrations"),
            ),
            "",
        ),
        ScriptEntry::fail(git(&repo, &format!("worktree remove --force {worktree}"))),
        ScriptEntry::ok(git(
            &repo,
            &format!("worktree add --detach {worktree} {SHA_A}"),
        )),
        // No cargo test / cargo build: all artifacts are reused.
        ScriptEntry::ok(format!(
            "cp -a {}/frontend/dist {}/frontend/dist",
            prior.display(),
            artifact.display()
        )),
        ScriptEntry::ok(format!("systemctl is-active --quiet {old_unit}")),
        ScriptEntry::ok(format!("systemctl start {new_unit}")),
        ScriptEntry::ok(format!("systemctl enable {new_unit}")),
        ScriptEntry::ok("systemctl restart sakiot-web.service"),
        ScriptEntry::ok("systemctl enable-web sakiot-web.service"),
        ScriptEntry::ok(format!("{worktree}/sakiot_stage/scripts/deploy.sh")),
        ScriptEntry::ok(format!("systemctl disable {old_unit}")),
        ScriptEntry::ok(format!("systemctl disable {prior_unit}")),
        ScriptEntry::ok(git(&repo, &format!("worktree remove --force {worktree}"))),
    ]);
    let events = Events::default();
    let admin = MockAdmin::ready(&events);
    let web = MockWeb {
        events: &events,
        healthy: true,
    };
    let clock = MockClock::new();

    world
        .run(
            &["rollback", "v1.0.0", SHA_A],
            &runner,
            &admin,
            &web,
            &clock,
        )
        .unwrap();
    runner.assert_exhausted().unwrap();

    // Binaries came from the prior release, not the cargo target dir.
    assert_eq!(
        fs::read_to_string(artifact.join("fbi-agent/fbi_agent")).unwrap(),
        "old-bot-bin"
    );
    assert_eq!(
        fs::read_to_string(artifact.join("web/web_server")).unwrap(),
        "old-web-bin"
    );
    let manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(artifact.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["mode"], "rollback");
    assert_eq!(
        manifest["reused"],
        serde_json::json!({"bot": true, "web": true, "frontend": true})
    );
    assert_eq!(
        manifest["components"],
        serde_json::json!(["bot", "web", "frontend"])
    );

    assert_eq!(
        events.all(),
        vec![
            format!("admin start_drain 127.0.0.1:40000 'deploy {release_id}'"),
            format!("admin drain_status 127.0.0.1:{PORT}"),
            format!(
                "web publish http://127.0.0.1:1/registry active=127.0.0.1:{PORT} draining=[\"127.0.0.1:40000\"]"
            ),
            format!("web health http://127.0.0.1:1/healthz {release_id}"),
            // The web restart wiped the in-memory registry; republished with
            // the drain still pending.
            format!(
                "web publish http://127.0.0.1:1/registry active=127.0.0.1:{PORT} draining=[\"127.0.0.1:40000\"]"
            ),
            format!(
                "admin shutdown_when_empty 127.0.0.1:40000 'release {release_id} is fully ready'"
            ),
        ]
    );
}

#[test]
fn bot_readiness_failure_cancels_drain_and_keeps_state() {
    let world = World::new();
    let repo = world.repo();
    let release_id = format!("staging-bbbbbbbbbbbb-{STAMP}");
    let worktree = world.worktree(&release_id).display().to_string();
    world.fake_worktree(&release_id);

    let old_unit = "sakiot-fbi-agent@old-release.service";
    world.write_state("current.sha", SHA_A);
    world.write_state("current.tag", "main");
    world.write_state("current-bot.unit", old_unit);
    world.write_state("current-bot.grpc", "127.0.0.1:40000");

    let new_unit = format!("sakiot-fbi-agent@{release_id}.service");
    let runner = ScriptedRunner::new(vec![
        ScriptEntry::ok(git(
            &repo,
            "remote set-url origin https://example.invalid/sakiot.git",
        )),
        ScriptEntry::ok(git(
            &repo,
            "fetch --prune origin +refs/heads/*:refs/remotes/origin/*",
        )),
        ScriptEntry::ok(git(&repo, &format!("cat-file -e {SHA_B}^{{commit}}"))),
        ScriptEntry::ok_with(
            git(&repo, &format!("diff --name-only {SHA_A} {SHA_B}")),
            "FBI-agent/src/main.rs\n",
        ),
        ScriptEntry::fail(git(&repo, &format!("worktree remove --force {worktree}"))),
        ScriptEntry::ok(git(
            &repo,
            &format!("worktree add --detach {worktree} {SHA_B}"),
        )),
        ScriptEntry::ok("cargo test --workspace --locked"),
        ScriptEntry::ok("cargo build --release --locked --package fbi_agent"),
        ScriptEntry::ok(format!("systemctl is-active --quiet {old_unit}")),
        ScriptEntry::ok(format!("systemctl start {new_unit}")),
        ScriptEntry::ok(format!("systemctl stop {new_unit}")),
        ScriptEntry::ok(git(&repo, &format!("worktree remove --force {worktree}"))),
    ]);
    let events = Events::default();
    let admin = MockAdmin::never_ready(&events);
    let web = MockWeb {
        events: &events,
        healthy: true,
    };
    let clock = MockClock::new();

    let error = world
        .run(&["stage", SHA_B], &runner, &admin, &web, &clock)
        .unwrap_err();
    assert_eq!(error.to_string(), "new FBI Agent failed readiness");
    runner.assert_exhausted().unwrap();

    // 30 readiness probes with a 1s pause after each, like the bash loop.
    assert_eq!(clock.sleep_count(), 30);

    // Old bot remains the recorded current instance; drain was cancelled;
    // the registry was never touched.
    assert_eq!(world.state("current-bot.unit").as_deref(), Some(old_unit));
    assert_eq!(
        world.state("current-bot.grpc").as_deref(),
        Some("127.0.0.1:40000")
    );
    assert_eq!(world.state("current.sha").as_deref(), Some(SHA_A));
    let all = events.all();
    assert_eq!(
        all[0],
        format!("admin start_drain 127.0.0.1:40000 'deploy {release_id}'")
    );
    assert_eq!(
        all.last().map(String::as_str),
        Some(
            format!("admin cancel_drain 127.0.0.1:40000 'release {release_id} failed readiness'")
                .as_str()
        )
    );
    assert!(!all.iter().any(|event| event.starts_with("web publish")));
}

#[test]
fn web_readiness_failure_restores_previous_symlink() {
    let world = World::new();
    let repo = world.repo();
    let release_id = format!("staging-bbbbbbbbbbbb-{STAMP}");
    let worktree = world.worktree(&release_id).display().to_string();
    world.fake_worktree(&release_id);
    world.write_state("current.sha", SHA_A);
    world.write_state("current.tag", "main");

    // A previous web release is live behind the symlink.
    let previous = world
        .config
        .release_root
        .join("staging-aaaaaaaaaaaa-20260101T120000Z");
    fs::create_dir_all(previous.join("web")).unwrap();
    fs::create_dir_all(&world.config.current_root).unwrap();
    std::os::unix::fs::symlink(previous.join("web"), world.config.current_root.join("web"))
        .unwrap();

    let runner = ScriptedRunner::new(vec![
        ScriptEntry::ok(git(
            &repo,
            "remote set-url origin https://example.invalid/sakiot.git",
        )),
        ScriptEntry::ok(git(
            &repo,
            "fetch --prune origin +refs/heads/*:refs/remotes/origin/*",
        )),
        ScriptEntry::ok(git(&repo, &format!("cat-file -e {SHA_B}^{{commit}}"))),
        ScriptEntry::ok_with(
            git(&repo, &format!("diff --name-only {SHA_A} {SHA_B}")),
            "web_server/src/main.rs\n",
        ),
        ScriptEntry::fail(git(&repo, &format!("worktree remove --force {worktree}"))),
        ScriptEntry::ok(git(
            &repo,
            &format!("worktree add --detach {worktree} {SHA_B}"),
        )),
        ScriptEntry::ok("cargo test --workspace --locked"),
        ScriptEntry::ok("cargo build --release --locked --package web_server"),
        ScriptEntry::ok("systemctl restart sakiot-web.service"),
        ScriptEntry::ok("systemctl enable-web sakiot-web.service"),
        ScriptEntry::ok("systemctl stop sakiot-web.service"),
        ScriptEntry::ok("systemctl restart sakiot-web.service"),
        ScriptEntry::ok(git(&repo, &format!("worktree remove --force {worktree}"))),
    ]);
    let events = Events::default();
    let admin = MockAdmin::ready(&events);
    let web = MockWeb {
        events: &events,
        healthy: false,
    };
    let clock = MockClock::new();

    let error = world
        .run(&["stage", SHA_B], &runner, &admin, &web, &clock)
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "web server failed readiness; previous release restored"
    );
    runner.assert_exhausted().unwrap();
    assert_eq!(clock.sleep_count(), 30);

    // Symlink restored to the previous release; state files untouched.
    assert_eq!(
        fs::read_link(world.config.current_root.join("web")).unwrap(),
        previous.join("web")
    );
    assert_eq!(world.state("current.sha").as_deref(), Some(SHA_A));
}

#[test]
fn rollback_crossing_migrations_is_rejected() {
    let world = World::new();
    let repo = world.repo();
    world.write_state("current.sha", SHA_B);
    world.write_state("current.tag", "v1.0.1");
    fs::create_dir_all(world.config.state_dir.join("tags")).unwrap();
    fs::write(
        world.config.state_dir.join("tags/v1.0.0"),
        format!("{SHA_A}\n"),
    )
    .unwrap();

    let runner = ScriptedRunner::new(vec![
        ScriptEntry::ok(git(
            &repo,
            "remote set-url origin https://example.invalid/sakiot.git",
        )),
        ScriptEntry::ok(git(
            &repo,
            "fetch --prune origin +refs/heads/*:refs/remotes/origin/* +refs/tags/v1.0.0:refs/tags/v1.0.0",
        )),
        ScriptEntry::ok_with(
            git(&repo, "rev-list -n 1 refs/tags/v1.0.0"),
            format!("{SHA_A}\n"),
        ),
        ScriptEntry::ok(git(&repo, &format!("cat-file -e {SHA_A}^{{commit}}"))),
        ScriptEntry::ok_with(
            git(
                &repo,
                &format!("diff --name-only {SHA_A} {SHA_B} -- sakiot-db/migrations"),
            ),
            "sakiot-db/migrations/0002_new.sql\n",
        ),
    ]);
    let events = Events::default();
    let admin = MockAdmin::ready(&events);
    let web = MockWeb {
        events: &events,
        healthy: true,
    };
    let clock = MockClock::new();

    let error = world
        .run(
            &["rollback", "v1.0.0", SHA_A],
            &runner,
            &admin,
            &web,
            &clock,
        )
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "rollback crosses migration changes; use explicit schema compatibility override"
    );
    runner.assert_exhausted().unwrap();
    assert!(events.all().is_empty());
    // Nothing was written: no release dir, unchanged state.
    assert_eq!(fs::read_dir(&world.config.release_root).unwrap().count(), 0);
    assert_eq!(world.state("current.sha").as_deref(), Some(SHA_B));
}
