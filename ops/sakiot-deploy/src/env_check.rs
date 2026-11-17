//! Checks an instance's env file against the example its release carries.
//!
//! `ops/<target>.env.example` lists every setting. An uncommented `KEY=value`
//! line is required: the env file must set `KEY` to something, or the
//! services start without it. A commented `# KEY=value` line documents an
//! optional setting and its default. The example is read at the commit being
//! deployed, so a release that adds a setting is refused, before anything is
//! built or restarted, until the server's env file has it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::config::{Config, Mode, Request, Target, assignment_key, plain_env_vars};
use crate::git;
use crate::log;
use crate::runner::CommandRunner;

/// The settings an example declares.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ExampleKeys {
    pub required: BTreeSet<String>,
    pub optional: BTreeSet<String>,
}

impl ExampleKeys {
    pub fn parse(example: &str) -> ExampleKeys {
        let mut keys = ExampleKeys::default();
        for line in example.lines() {
            let line = line.trim();
            if let Some(comment) = line.strip_prefix('#') {
                if let Some(key) = assignment_key(comment.trim_start()) {
                    keys.optional.insert(key.to_string());
                }
            } else if let Some(key) = assignment_key(line) {
                keys.required.insert(key.to_string());
            }
        }
        keys
    }

    fn mentions(&self, key: &str) -> bool {
        self.required.contains(key) || self.optional.contains(key)
    }
}

/// How an env file differs from its example.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Drift {
    /// Required by the example, but unset or empty in the env file.
    pub missing: Vec<String>,
    /// Set in the env file but absent from the example: a typo, or a setting
    /// that no longer exists.
    pub unknown: Vec<String>,
}

pub fn drift(example: &ExampleKeys, env: &BTreeMap<String, String>) -> Drift {
    Drift {
        missing: example
            .required
            .iter()
            .filter(|key| {
                env.get(key.as_str())
                    .is_none_or(|value| value.trim().is_empty())
            })
            .cloned()
            .collect(),
        unknown: env
            .keys()
            .filter(|key| !example.mentions(key))
            .cloned()
            .collect(),
    }
}

/// Compares the deploy's env file with the example at the deployed commit,
/// and refuses the deploy when a required setting is missing. A rollback only
/// warns: putting back what ran before must not wait on tidying settings. A
/// staging deploy also checks production's env file and warns, so a setting
/// the next release needs shows up on the push that adds it.
pub fn check(
    runner: &dyn CommandRunner,
    source_repo: &Path,
    request: &Request,
    config: &Config,
) -> Result<()> {
    let sha = request.sha.as_str();
    if request.target == Target::Staging {
        let production_env = &config.production_env_file;
        match read_drift(runner, source_repo, sha, Target::Production, production_env) {
            Ok(drift) => {
                warn_unknown(production_env, Target::Production, &drift.unknown);
                if !drift.missing.is_empty() {
                    log(format!(
                        "WARNING: {} is missing {}, which {} requires at {}; \
                         releasing this commit will fail until it is set",
                        production_env.display(),
                        list(&drift.missing),
                        example_path(Target::Production),
                        short(sha)
                    ));
                }
            }
            Err(error) => log(format!(
                "WARNING: production env file check skipped: {error:#}"
            )),
        }
    }

    let target = request.target;
    let env_file = &config.env_file;
    let drift = match read_drift(runner, source_repo, sha, target, env_file) {
        Ok(drift) => drift,
        Err(error) if request.mode == Mode::Rollback => {
            log(format!("WARNING: env file check skipped: {error:#}"));
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    warn_unknown(env_file, target, &drift.unknown);
    if drift.missing.is_empty() {
        log(format!(
            "{} has every setting {} requires",
            env_file.display(),
            example_path(target)
        ));
        return Ok(());
    }
    let missing = format!(
        "{} is missing {}, which {} requires at {}",
        env_file.display(),
        list(&drift.missing),
        example_path(target),
        short(sha)
    );
    if request.mode == Mode::Rollback {
        log(format!("WARNING: {missing}; rolling back anyway"));
        return Ok(());
    }
    let them = if drift.missing.len() == 1 {
        "it"
    } else {
        "them"
    };
    bail!("{missing}. Add {them} to the env file and run the deploy again; nothing was deployed")
}

fn read_drift(
    runner: &dyn CommandRunner,
    source_repo: &Path,
    sha: &str,
    target: Target,
    env_file: &Path,
) -> Result<Drift> {
    let path = example_path(target);
    let example = git::show_file(runner, source_repo, sha, &path)
        .with_context(|| format!("failed to read {path} at {}", short(sha)))?;
    let env = plain_env_vars(env_file)?;
    Ok(drift(&ExampleKeys::parse(&example), &env))
}

fn example_path(target: Target) -> String {
    format!("ops/{}.env.example", target.as_str())
}

fn warn_unknown(env_file: &Path, target: Target, unknown: &[String]) {
    if !unknown.is_empty() {
        log(format!(
            "WARNING: {} sets {}, which {} does not mention: a typo, or a setting that no longer exists",
            env_file.display(),
            list(unknown),
            example_path(target)
        ));
    }
}

/// "A", "A and B", "A, B and C".
fn list(keys: &[String]) -> String {
    match keys {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn short(sha: &str) -> &str {
    sha.get(..12).unwrap_or(sha)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set<const N: usize>(keys: [&str; N]) -> BTreeSet<String> {
        keys.into_iter().map(String::from).collect()
    }

    fn env<const N: usize>(pairs: [(&str, &str); N]) -> BTreeMap<String, String> {
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn uncommented_settings_are_required_and_commented_ones_optional() {
        let keys = ExampleKeys::parse(
            "# Prose is skipped, even with ?mode=permanent in it.\n\
             DATABASE_URL=postgres://x\n\
             export PORT=8900\n\
             # SAKIOT_KEEP_RELEASES=5\n\
             #SAKIOT_SKIP_DB_BACKUP=0\n\
             \n",
        );
        assert_eq!(keys.required, set(["DATABASE_URL", "PORT"]));
        assert_eq!(
            keys.optional,
            set(["SAKIOT_KEEP_RELEASES", "SAKIOT_SKIP_DB_BACKUP"])
        );
    }

    #[test]
    fn a_required_setting_must_be_set_to_something() {
        let keys = ExampleKeys::parse("A=1\nB=2\nC=3\nD=4\n");
        let drift = drift(&keys, &env([("A", "x"), ("B", ""), ("C", "  ")]));
        assert_eq!(drift.missing, ["B", "C", "D"]);
        assert!(drift.unknown.is_empty());
    }

    #[test]
    fn optional_settings_may_be_left_out_or_set() {
        let keys = ExampleKeys::parse("A=1\n# B=2\n# C=3\n");
        assert_eq!(
            drift(&keys, &env([("A", "x"), ("B", "y")])),
            Drift::default()
        );
    }

    #[test]
    fn settings_the_example_does_not_mention_are_reported() {
        let keys = ExampleKeys::parse("SAKIOT_FLAG_ENABLED=true\n");
        let drift = drift(
            &keys,
            &env([("SAKIOT_FLAG_ENABLE", "true"), ("REMOVED_SETTING", "1")]),
        );
        assert_eq!(drift.missing, ["SAKIOT_FLAG_ENABLED"]);
        assert_eq!(drift.unknown, ["REMOVED_SETTING", "SAKIOT_FLAG_ENABLE"]);
    }

    #[test]
    fn keys_are_listed_as_prose() {
        let keys = |names: &[&str]| names.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(list(&keys(&["A"])), "A");
        assert_eq!(list(&keys(&["A", "B"])), "A and B");
        assert_eq!(list(&keys(&["A", "B", "C"])), "A, B and C");
    }

    /// Every line of the real examples must be one the check understands, or
    /// a setting could hide from it.
    #[test]
    fn the_examples_declare_every_setting_readably() {
        let examples = [
            ("production", include_str!("../../production.env.example")),
            ("staging", include_str!("../../staging.env.example")),
            ("preview", include_str!("../../preview.env.example")),
        ];
        for (name, example) in examples {
            for (number, line) in example.lines().enumerate() {
                let line = line.trim();
                assert!(
                    line.is_empty() || line.starts_with('#') || assignment_key(line).is_some(),
                    "{name}.env.example line {} is not KEY=value",
                    number + 1
                );
            }
            let keys = ExampleKeys::parse(example);
            assert!(keys.required.contains("DATABASE_URL"), "{name}");
            assert!(
                keys.required.contains("FBI_AGENT_REGISTRY_SECRET"),
                "{name}"
            );
        }
        // Both went missing from the production server once.
        let production = ExampleKeys::parse(examples[0].1);
        assert!(
            production
                .required
                .contains("SAKIOT_RECORDING_PERMANENT_DELETE_ENABLED")
        );
    }
}
