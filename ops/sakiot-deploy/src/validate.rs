//! Validators ported from ops/lib/common.sh. All failures mirror the bash
//! `die` messages so operator-facing errors stay identical.

use std::path::Path;

use anyhow::{Result, bail};

use crate::config::Mode;

/// Strict semver. Suffixes/typos (v1.23, v1.2.3-rc1) are rejected so only
/// intentional production releases deploy.
pub fn validate_tag(tag: &str) -> Result<()> {
    let Some(rest) = tag.strip_prefix('v') else {
        bail!("invalid release tag: {tag}");
    };
    let parts: Vec<&str> = rest.split('.').collect();
    let valid = parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    if !valid {
        bail!("invalid release tag: {tag}");
    }
    Ok(())
}

pub fn validate_sha(sha: &str) -> Result<()> {
    let valid = sha.len() == 40
        && sha
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if !valid {
        bail!("invalid commit SHA: {sha}");
    }
    Ok(())
}

pub fn database_name_from_url(url: &str) -> Result<String> {
    let without_query = url.split('?').next().unwrap_or(url);
    let database = without_query.rsplit('/').next().unwrap_or(without_query);

    if !(url.starts_with("postgres://") || url.starts_with("postgresql://")) {
        bail!("database URL must use postgres:// or postgresql://");
    }
    if database.is_empty() || database == without_query {
        bail!("database URL must include a database name");
    }
    Ok(database.to_string())
}

pub fn validate_test_database_url(runtime_url: &str, test_url: &str) -> Result<()> {
    if test_url.is_empty() {
        bail!("SAKIOT_TEST_DATABASE_URL must be set");
    }
    let runtime_database = database_name_from_url(runtime_url)?;
    let test_database = database_name_from_url(test_url)?;
    if !test_database.ends_with("_test") {
        bail!("SAKIOT_TEST_DATABASE_URL database must end in _test");
    }
    if test_database == runtime_database {
        bail!("test and runtime database names must differ");
    }
    Ok(())
}

pub fn validate_tag_record(mode: Mode, record: &Path, tag: &str, sha: &str) -> Result<()> {
    match mode {
        Mode::Release => {
            if record.exists() {
                let recorded_sha = std::fs::read_to_string(record)?;
                let recorded_sha = recorded_sha.trim_end_matches('\n');
                if recorded_sha == sha {
                    bail!("release tag already deployed successfully: {tag}");
                }
                bail!("release tag was moved from {recorded_sha} to {sha}");
            }
        }
        Mode::Rollback => {
            if !record.exists() {
                bail!("rollback tag was not previously deployed: {tag}");
            }
            let recorded_sha = std::fs::read_to_string(record)?;
            if recorded_sha.trim_end_matches('\n') != sha {
                bail!("rollback tag record does not match supplied SHA");
            }
        }
        Mode::Stage => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Ported from ops/tests/validation_test.sh.

    use super::*;

    const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[test]
    fn tag_accepts_strict_semver() {
        for tag in ["v0.0.1", "v1.2.3", "v10.20.30"] {
            assert!(validate_tag(tag).is_ok(), "{tag} should be valid");
        }
    }

    #[test]
    fn tag_rejects_non_semver() {
        for tag in [
            "1.2.3",
            "v1.2",
            "v1.2.3-rc1",
            "v1.2.3.4",
            "v1..3",
            "main",
            "",
            "v1.2.x",
        ] {
            assert!(validate_tag(tag).is_err(), "{tag} should be rejected");
        }
    }

    #[test]
    fn sha_accepts_forty_hex() {
        assert!(validate_sha(SHA_A).is_ok());
        assert!(validate_sha("0123456789abcdef0123456789abcdef01234567").is_ok());
    }

    #[test]
    fn sha_rejects_invalid() {
        for sha in [
            "",
            "abc",
            &SHA_A[..39],
            &format!("{SHA_A}a"),
            "ABCDEF0123456789ABCDEF0123456789ABCDEF01",
            "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
        ] {
            assert!(validate_sha(sha).is_err(), "{sha} should be rejected");
        }
    }

    #[test]
    fn database_name_extraction() {
        assert_eq!(
            database_name_from_url("postgres://u:p@localhost:5432/sakiot_rouvas").unwrap(),
            "sakiot_rouvas"
        );
        assert_eq!(
            database_name_from_url("postgresql://localhost/db?sslmode=disable").unwrap(),
            "db"
        );
        assert!(database_name_from_url("mysql://localhost/db").is_err());
        assert!(database_name_from_url("postgres://localhost:5432/").is_err());
    }

    #[test]
    fn test_database_url_rules() {
        let runtime = "postgres://localhost/sakiot_rouvas";
        assert!(validate_test_database_url(runtime, "postgres://localhost/sakiot_test").is_ok());
        assert!(validate_test_database_url(runtime, "").is_err());
        assert!(
            validate_test_database_url(runtime, "postgres://localhost/sakiot_staging").is_err(),
            "test database must end in _test"
        );
        assert!(
            validate_test_database_url(
                "postgres://localhost/sakiot_test",
                "postgres://localhost/sakiot_test"
            )
            .is_err(),
            "test and runtime names must differ"
        );
    }

    #[test]
    fn tag_record_release_rules() {
        let dir = tempfile::tempdir().expect("tempdir");
        let record = dir.path().join("v1.0.0");

        // Fresh tag: fine.
        validate_tag_record(Mode::Release, &record, "v1.0.0", SHA_A).expect("fresh tag");

        std::fs::write(&record, format!("{SHA_A}\n")).expect("write record");
        let same = validate_tag_record(Mode::Release, &record, "v1.0.0", SHA_A);
        assert!(
            same.unwrap_err()
                .to_string()
                .contains("already deployed successfully")
        );
        let moved = validate_tag_record(Mode::Release, &record, "v1.0.0", SHA_B);
        assert!(moved.unwrap_err().to_string().contains("was moved from"));
    }

    #[test]
    fn tag_record_rollback_rules() {
        let dir = tempfile::tempdir().expect("tempdir");
        let record = dir.path().join("v1.0.0");

        let missing = validate_tag_record(Mode::Rollback, &record, "v1.0.0", SHA_A);
        assert!(
            missing
                .unwrap_err()
                .to_string()
                .contains("not previously deployed")
        );

        std::fs::write(&record, format!("{SHA_A}\n")).expect("write record");
        validate_tag_record(Mode::Rollback, &record, "v1.0.0", SHA_A).expect("matching record");
        let mismatch = validate_tag_record(Mode::Rollback, &record, "v1.0.0", SHA_B);
        assert!(
            mismatch
                .unwrap_err()
                .to_string()
                .contains("does not match supplied SHA")
        );
    }
}
