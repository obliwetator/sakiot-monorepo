//! Git cache and worktree management (deploy-release.sh lines 86-140).

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use crate::log;
use crate::runner::{Cmd, CommandRunner};

fn git(source_repo: &Path) -> Cmd {
    Cmd::new("git")
        .arg("-C")
        .arg(source_repo.display().to_string())
}

pub fn ensure_cache(runner: &dyn CommandRunner, source_repo: &Path, url: &str) -> Result<()> {
    if !source_repo.join(".git").is_dir() {
        log("creating deployment repository cache");
        runner.run(
            &Cmd::new("git")
                .arg("clone")
                .arg("--no-checkout")
                .arg(url)
                .arg(source_repo.display().to_string()),
        )?;
    }
    runner.run(&git(source_repo).args(["remote", "set-url", "origin", url]))
}

pub fn fetch_production(runner: &dyn CommandRunner, source_repo: &Path, tag: &str) -> Result<()> {
    runner.run(&git(source_repo).args([
        "fetch",
        "--prune",
        "origin",
        "+refs/heads/*:refs/remotes/origin/*",
        &format!("+refs/tags/{tag}:refs/tags/{tag}"),
    ]))
}

pub fn fetch_staging(runner: &dyn CommandRunner, source_repo: &Path) -> Result<()> {
    runner.run(&git(source_repo).args([
        "fetch",
        "--prune",
        "origin",
        "+refs/heads/*:refs/remotes/origin/*",
    ]))
}

pub fn resolve_tag(runner: &dyn CommandRunner, source_repo: &Path, tag: &str) -> Result<String> {
    let output = runner.run_capture(&git(source_repo).args([
        "rev-list",
        "-n",
        "1",
        &format!("refs/tags/{tag}"),
    ]))?;
    Ok(output.trim().to_string())
}

pub fn require_commit(runner: &dyn CommandRunner, source_repo: &Path, sha: &str) -> Result<()> {
    runner.run(&git(source_repo).args(["cat-file", "-e", &format!("{sha}^{{commit}}")]))
}

/// `git diff --name-only <from> <to> [-- <pathspec>]`.
pub fn diff_names(
    runner: &dyn CommandRunner,
    source_repo: &Path,
    from: &str,
    to: &str,
    pathspec: Option<&str>,
) -> Result<Vec<String>> {
    let mut cmd = git(source_repo).args(["diff", "--name-only", from, to]);
    if let Some(pathspec) = pathspec {
        cmd = cmd.arg("--").arg(pathspec);
    }
    let output = runner.run_capture(&cmd)?;
    Ok(output
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

/// Detached worktree for the release SHA. Removal mirrors the bash EXIT trap:
/// best-effort `git worktree remove --force`, also run before `add` to clear
/// any leftover directory from an interrupted run.
pub struct Worktree<'a> {
    runner: &'a dyn CommandRunner,
    source_repo: PathBuf,
    path: PathBuf,
}

impl<'a> Worktree<'a> {
    pub fn add(
        runner: &'a dyn CommandRunner,
        source_repo: &Path,
        path: &Path,
        sha: &str,
    ) -> Result<Worktree<'a>> {
        let worktree = Worktree {
            runner,
            source_repo: source_repo.to_path_buf(),
            path: path.to_path_buf(),
        };
        worktree.remove_best_effort();
        runner.run(&git(source_repo).args([
            "worktree",
            "add",
            "--detach",
            &path.display().to_string(),
            sha,
        ]))?;
        Ok(worktree)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn remove_best_effort(&self) {
        let _ = self.runner.run_ok(&git(&self.source_repo).args([
            "worktree",
            "remove",
            "--force",
            &self.path.display().to_string(),
        ]));
    }
}

impl Drop for Worktree<'_> {
    fn drop(&mut self) {
        self.remove_best_effort();
    }
}

/// `find <migrations> -maxdepth 1 -type f -printf '%f\n' | sort | tail -n 1`.
pub fn migration_head(migrations_dir: &Path) -> Result<String> {
    let mut names: Vec<String> = Vec::new();
    let entries = match std::fs::read_dir(migrations_dir) {
        Ok(entries) => entries,
        Err(error) => bail!(
            "failed to read migrations directory {}: {error}",
            migrations_dir.display()
        ),
    };
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    names.sort();
    Ok(names.pop().unwrap_or_default())
}
