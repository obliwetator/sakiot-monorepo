//! Filesystem tests for artifact reuse and release garbage collection,
//! ported from ops/tests/reuse_artifact_test.sh and release_gc_test.sh.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

use sakiot_deploy::components::Component;
use sakiot_deploy::release::{prune_old_releases, reusable_artifact};
use sakiot_deploy::runner::{ScriptEntry, ScriptedRunner};
use sakiot_deploy::systemctl::Systemctl;

const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

/// Fabricate a release dir with a manifest and chosen artifacts.
fn make_release(releases: &Path, id: &str, sha: &str, artifacts: &[&str]) -> PathBuf {
    let dir = releases.join(id);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("manifest.json"),
        format!("{{\"sha\":\"{sha}\"}}\n"),
    )
    .unwrap();
    for artifact in artifacts {
        match *artifact {
            "bot" => {
                fs::create_dir_all(dir.join("fbi-agent")).unwrap();
                fs::write(dir.join("fbi-agent/fbi_agent"), "bin\n").unwrap();
            }
            "web" => {
                fs::create_dir_all(dir.join("web")).unwrap();
                fs::write(dir.join("web/web_server"), "bin\n").unwrap();
            }
            "frontend" => {
                fs::create_dir_all(dir.join("frontend/dist")).unwrap();
                fs::write(dir.join("frontend/dist/index.html"), "x\n").unwrap();
            }
            "bot-empty" => {
                fs::create_dir_all(dir.join("fbi-agent")).unwrap();
                fs::write(dir.join("fbi-agent/fbi_agent"), "").unwrap();
            }
            "frontend-empty" => {
                fs::create_dir_all(dir.join("frontend/dist")).unwrap();
            }
            other => panic!("unknown artifact kind {other}"),
        }
    }
    dir
}

fn reuse(releases: &Path, sha: &str, component: Component, exclude: &Path) -> Option<PathBuf> {
    reusable_artifact(releases, sha, component, exclude).unwrap()
}

#[test]
fn reusable_artifact_matches_bash_suite() {
    let root = tempfile::tempdir().unwrap();
    let releases = root.path().join("releases");
    fs::create_dir_all(&releases).unwrap();
    let releases = releases.canonicalize().unwrap();
    let none = PathBuf::new();

    // Oldest -> newest by timestamp in the id.
    let old_a = make_release(
        &releases,
        "v1.0.1-aaaaaaaaaaaa-20260101T120000Z",
        SHA_A,
        &["bot"],
    );
    let new_a = make_release(
        &releases,
        "v1.0.1-aaaaaaaaaaaa-20260102T120000Z",
        SHA_A,
        &["bot", "web", "frontend"],
    );
    let _other_b = make_release(
        &releases,
        "v1.0.2-bbbbbbbbbbbb-20260103T120000Z",
        SHA_B,
        &["bot", "web", "frontend"],
    );

    // 1. newest matching dir for the SHA is chosen for each component
    assert_eq!(
        reuse(&releases, SHA_A, Component::Bot, &none).as_ref(),
        Some(&new_a)
    );
    assert_eq!(
        reuse(&releases, SHA_A, Component::Web, &none).as_ref(),
        Some(&new_a)
    );
    assert_eq!(
        reuse(&releases, SHA_A, Component::Frontend, &none).as_ref(),
        Some(&new_a)
    );

    // 2. excluding the newest falls back to the older dir that still has the
    // component; older A has no web -> rebuild.
    assert_eq!(
        reuse(&releases, SHA_A, Component::Bot, &new_a).as_ref(),
        Some(&old_a)
    );
    assert_eq!(reuse(&releases, SHA_A, Component::Web, &new_a), None);

    // 3. unknown SHA -> empty
    assert_eq!(reuse(&releases, "deadbeef", Component::Bot, &none), None);

    // 4. other SHA's artifacts are not used for SHA_A
    assert_eq!(
        reuse(&releases, SHA_A, Component::Bot, &new_a).as_ref(),
        Some(&old_a)
    );

    // 5. empty artifact files / dirs are not matched
    make_release(
        &releases,
        "v1.0.3-cccccccccccc-20260104T120000Z",
        SHA_A,
        &["bot-empty", "frontend-empty"],
    );
    assert_eq!(
        reuse(&releases, SHA_A, Component::Bot, &none).as_ref(),
        Some(&new_a)
    );
    assert_eq!(
        reuse(&releases, SHA_A, Component::Frontend, &none).as_ref(),
        Some(&new_a)
    );
    assert_eq!(reuse(&releases, SHA_A, Component::Frontend, &new_a), None);
}

const PROD_PREFIX: &str = "sakiot-fbi-agent@";
const STAGING_PREFIX: &str = "sakiot-staging-fbi-agent@";

/// Fake deployment tree: 7 releases with increasing timestamps, each with an
/// fbi-agent/ subdir; current points at the newest.
fn make_tree(root: &Path, prefix: &str) -> (PathBuf, PathBuf, PathBuf) {
    let release_root = root.join("releases");
    let current_root = root.join("current");
    let state_dir = root.join("state");
    for dir in [&release_root, &current_root, &state_dir] {
        fs::create_dir_all(dir).unwrap();
    }

    for i in 1..=7 {
        let id = format!("v1.0.{i}-abc123def456-2026010{i}T120000Z");
        fs::create_dir_all(release_root.join(&id).join("fbi-agent")).unwrap();
        fs::create_dir_all(release_root.join(&id).join("web")).unwrap();
        fs::write(release_root.join(&id).join("web/web_server"), "binary\n").unwrap();
    }

    let newest = "v1.0.7-abc123def456-20260107T120000Z";
    std::os::unix::fs::symlink(
        release_root.join(newest).join("web"),
        current_root.join("web"),
    )
    .unwrap();
    fs::write(
        state_dir.join("current.manifest"),
        format!(
            "{}\n",
            release_root.join(newest).join("manifest.json").display()
        ),
    )
    .unwrap();
    fs::write(
        state_dir.join("current-bot.unit"),
        format!("{prefix}{newest}.service\n"),
    )
    .unwrap();
    (release_root, current_root, state_dir)
}

fn unit(prefix: &str, release: &str) -> String {
    format!("{prefix}v1.0.{release}-abc123def456-2026010{release}T120000Z.service")
}

#[test]
fn prune_keeps_newest_five_when_nothing_drains() {
    let root = tempfile::tempdir().unwrap();
    let (release_root, current_root, state_dir) = make_tree(root.path(), PROD_PREFIX);

    // Candidates beyond keep=5, newest first: v1.0.2 then v1.0.1. Nothing is
    // active, so both are disabled and removed.
    let runner = ScriptedRunner::new(vec![
        ScriptEntry::fail(format!(
            "systemctl is-active --quiet {}",
            unit(PROD_PREFIX, "2")
        )),
        ScriptEntry::ok(format!("systemctl disable {}", unit(PROD_PREFIX, "2"))),
        ScriptEntry::fail(format!(
            "systemctl is-active --quiet {}",
            unit(PROD_PREFIX, "1")
        )),
        ScriptEntry::ok(format!("systemctl disable {}", unit(PROD_PREFIX, "1"))),
    ]);
    let systemctl = Systemctl::new(&runner, false);

    prune_old_releases(
        &systemctl,
        &release_root,
        &current_root,
        &state_dir,
        "5",
        PROD_PREFIX,
    )
    .unwrap();
    runner.assert_exhausted().unwrap();

    let remaining = fs::read_dir(&release_root).unwrap().count();
    assert_eq!(remaining, 5, "expected 5 releases kept");
    assert!(
        release_root
            .join("v1.0.7-abc123def456-20260107T120000Z")
            .is_dir()
    );
    assert!(
        !release_root
            .join("v1.0.1-abc123def456-20260101T120000Z")
            .exists()
    );
    assert!(
        !release_root
            .join("v1.0.2-abc123def456-20260102T120000Z")
            .exists()
    );
}

#[test]
fn prune_keeps_draining_release() {
    let root = tempfile::tempdir().unwrap();
    let (release_root, current_root, state_dir) = make_tree(root.path(), PROD_PREFIX);

    // v1.0.1 is still draining (is-active succeeds); v1.0.2 is pruned.
    let runner = ScriptedRunner::new(vec![
        ScriptEntry::fail(format!(
            "systemctl is-active --quiet {}",
            unit(PROD_PREFIX, "2")
        )),
        ScriptEntry::ok(format!("systemctl disable {}", unit(PROD_PREFIX, "2"))),
        ScriptEntry::ok(format!(
            "systemctl is-active --quiet {}",
            unit(PROD_PREFIX, "1")
        )),
    ]);
    let systemctl = Systemctl::new(&runner, false);

    prune_old_releases(
        &systemctl,
        &release_root,
        &current_root,
        &state_dir,
        "5",
        PROD_PREFIX,
    )
    .unwrap();
    runner.assert_exhausted().unwrap();

    assert!(
        release_root
            .join("v1.0.1-abc123def456-20260101T120000Z")
            .is_dir(),
        "draining release v1.0.1 must be kept despite being beyond keep=5"
    );
    assert!(
        !release_root
            .join("v1.0.2-abc123def456-20260102T120000Z")
            .exists()
    );
}

#[test]
fn prune_uses_configured_staging_unit_prefix() {
    let root = tempfile::tempdir().unwrap();
    let (release_root, current_root, state_dir) = make_tree(root.path(), STAGING_PREFIX);

    // Staging units must be queried (and a draining one kept) under the
    // staging prefix; the bash engine hardcoded the production prefix here.
    let runner = ScriptedRunner::new(vec![
        ScriptEntry::fail(format!(
            "systemctl is-active --quiet {}",
            unit(STAGING_PREFIX, "2")
        )),
        ScriptEntry::ok(format!("systemctl disable {}", unit(STAGING_PREFIX, "2"))),
        ScriptEntry::ok(format!(
            "systemctl is-active --quiet {}",
            unit(STAGING_PREFIX, "1")
        )),
    ]);
    let systemctl = Systemctl::new(&runner, false);

    prune_old_releases(
        &systemctl,
        &release_root,
        &current_root,
        &state_dir,
        "5",
        STAGING_PREFIX,
    )
    .unwrap();
    runner.assert_exhausted().unwrap();

    assert!(
        release_root
            .join("v1.0.1-abc123def456-20260101T120000Z")
            .is_dir(),
        "draining staging release v1.0.1 must be kept"
    );
    assert!(
        !release_root
            .join("v1.0.2-abc123def456-20260102T120000Z")
            .exists()
    );
}

#[test]
fn prune_skips_on_invalid_keep() {
    let root = tempfile::tempdir().unwrap();
    let (release_root, current_root, state_dir) = make_tree(root.path(), PROD_PREFIX);
    let runner = ScriptedRunner::new(vec![]);
    let systemctl = Systemctl::new(&runner, false);

    for keep in ["abc", "+5"] {
        prune_old_releases(
            &systemctl,
            &release_root,
            &current_root,
            &state_dir,
            keep,
            PROD_PREFIX,
        )
        .unwrap();
    }
    assert_eq!(fs::read_dir(&release_root).unwrap().count(), 7);
}
