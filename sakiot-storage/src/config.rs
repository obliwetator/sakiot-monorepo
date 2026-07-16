use std::{env, str::FromStr};

pub const DEFAULT_RETENTION_DAYS: u64 = 7;
pub const DEFAULT_CACHE_MAX_BYTES: u64 = 50 * 1024 * 1024 * 1024;

#[derive(Clone)]
pub struct ArchiveConfig {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub local_retention_days: u64,
    pub local_cache_max_bytes: u64,
    pub local_prune_enabled: bool,
}

impl std::fmt::Debug for ArchiveConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ArchiveConfig")
            .field("endpoint", &self.endpoint)
            .field("region", &self.region)
            .field("bucket", &self.bucket)
            .field("access_key_id", &"[redacted]")
            .field("secret_access_key", &"[redacted]")
            .field("local_retention_days", &self.local_retention_days)
            .field("local_cache_max_bytes", &self.local_cache_max_bytes)
            .field("local_prune_enabled", &self.local_prune_enabled)
            .finish()
    }
}

#[derive(Clone, Debug)]
pub enum ArchiveMode {
    Disabled,
    Enabled(ArchiveConfig),
}

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum ConfigError {
    #[error("{key} must be one of 0, 1, false, true, no, yes, off, or on; got {value}")]
    InvalidBoolean { key: &'static str, value: String },
    #[error("missing required environment variable: {0}")]
    Missing(&'static str),
    #[error("{key} must be a positive integer; got {value}")]
    InvalidPositiveInteger { key: &'static str, value: String },
    #[error(
        "SAKIOT_MEDIA_S3_ENDPOINT must be the HTTPS EU Central Backblaze S3 origin matching SAKIOT_MEDIA_S3_REGION"
    )]
    InvalidEndpoint,
    #[error("SAKIOT_MEDIA_S3_BUCKET must be a valid S3 bucket name")]
    InvalidBucket,
}

impl ArchiveMode {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|key| env::var(key).ok())
    }

    pub fn from_lookup(
        mut lookup: impl FnMut(&str) -> Option<String>,
    ) -> Result<Self, ConfigError> {
        let enabled = parse_bool(
            "SAKIOT_MEDIA_ARCHIVE_ENABLED",
            lookup("SAKIOT_MEDIA_ARCHIVE_ENABLED")
                .as_deref()
                .unwrap_or("0"),
        )?;
        if !enabled {
            return Ok(Self::Disabled);
        }

        let region = required(&mut lookup, "SAKIOT_MEDIA_S3_REGION")?;
        let endpoint = required(&mut lookup, "SAKIOT_MEDIA_S3_ENDPOINT")?;
        validate_endpoint(&endpoint, &region)?;
        let bucket = required(&mut lookup, "SAKIOT_MEDIA_S3_BUCKET")?;
        if !valid_bucket(&bucket) {
            return Err(ConfigError::InvalidBucket);
        }

        Ok(Self::Enabled(ArchiveConfig {
            endpoint,
            region,
            bucket,
            access_key_id: required(&mut lookup, "SAKIOT_MEDIA_S3_ACCESS_KEY_ID")?,
            secret_access_key: required(&mut lookup, "SAKIOT_MEDIA_S3_SECRET_ACCESS_KEY")?,
            local_retention_days: positive_integer(
                &mut lookup,
                "SAKIOT_MEDIA_LOCAL_RETENTION_DAYS",
                DEFAULT_RETENTION_DAYS,
            )?,
            local_cache_max_bytes: positive_integer(
                &mut lookup,
                "SAKIOT_MEDIA_LOCAL_CACHE_MAX_BYTES",
                DEFAULT_CACHE_MAX_BYTES,
            )?,
            local_prune_enabled: parse_bool(
                "SAKIOT_MEDIA_LOCAL_PRUNE_ENABLED",
                lookup("SAKIOT_MEDIA_LOCAL_PRUNE_ENABLED")
                    .as_deref()
                    .unwrap_or("0"),
            )?,
        }))
    }
}

fn required(
    lookup: &mut impl FnMut(&str) -> Option<String>,
    key: &'static str,
) -> Result<String, ConfigError> {
    lookup(key)
        .filter(|value| !value.trim().is_empty())
        .ok_or(ConfigError::Missing(key))
}

fn parse_bool(key: &'static str, value: &str) -> Result<bool, ConfigError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(ConfigError::InvalidBoolean {
            key,
            value: value.to_owned(),
        }),
    }
}

fn positive_integer(
    lookup: &mut impl FnMut(&str) -> Option<String>,
    key: &'static str,
    default: u64,
) -> Result<u64, ConfigError> {
    let Some(value) = lookup(key) else {
        return Ok(default);
    };
    u64::from_str(value.trim())
        .ok()
        .filter(|parsed| *parsed > 0)
        .ok_or(ConfigError::InvalidPositiveInteger { key, value })
}

fn validate_endpoint(endpoint: &str, region: &str) -> Result<(), ConfigError> {
    let url = url::Url::parse(endpoint).map_err(|_| ConfigError::InvalidEndpoint)?;
    let expected_host = format!("s3.{region}.backblazeb2.com");
    if !region.starts_with("eu-central-")
        || url.scheme() != "https"
        || url.host_str() != Some(expected_host.as_str())
        || url.port().is_some_and(|port| port != 443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/")
    {
        return Err(ConfigError::InvalidEndpoint);
    }
    Ok(())
}

fn valid_bucket(bucket: &str) -> bool {
    (6..=63).contains(&bucket.len())
        && bucket
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && bucket
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && bucket
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        && !bucket.contains("--")
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn enabled() -> HashMap<String, String> {
        [
            ("SAKIOT_MEDIA_ARCHIVE_ENABLED", "1"),
            (
                "SAKIOT_MEDIA_S3_ENDPOINT",
                "https://s3.eu-central-003.backblazeb2.com",
            ),
            ("SAKIOT_MEDIA_S3_REGION", "eu-central-003"),
            ("SAKIOT_MEDIA_S3_BUCKET", "sakiot-media-staging-abc123"),
            ("SAKIOT_MEDIA_S3_ACCESS_KEY_ID", "key-id"),
            (
                "SAKIOT_MEDIA_S3_SECRET_ACCESS_KEY",
                "secret-value-should-not-appear",
            ),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
    }

    #[test]
    fn disabled_needs_no_remote_values() {
        let mode = ArchiveMode::from_lookup(|_| None);
        assert!(matches!(mode, Ok(ArchiveMode::Disabled)));
    }

    #[test]
    fn enabled_requires_every_credential() {
        let mut values = enabled();
        values.remove("SAKIOT_MEDIA_S3_SECRET_ACCESS_KEY");
        assert!(matches!(
            ArchiveMode::from_lookup(|key| values.get(key).cloned()),
            Err(ConfigError::Missing("SAKIOT_MEDIA_S3_SECRET_ACCESS_KEY"))
        ));
    }

    #[test]
    fn enabled_rejects_non_https_endpoint() {
        let mut values = enabled();
        values.insert(
            "SAKIOT_MEDIA_S3_ENDPOINT".to_owned(),
            "http://localhost:9000".to_owned(),
        );
        assert!(matches!(
            ArchiveMode::from_lookup(|key| values.get(key).cloned()),
            Err(ConfigError::InvalidEndpoint)
        ));
    }

    #[test]
    fn enabled_rejects_endpoint_region_mismatch_and_short_bucket() {
        let mut values = enabled();
        values.insert(
            "SAKIOT_MEDIA_S3_REGION".to_owned(),
            "eu-central-004".to_owned(),
        );
        assert!(matches!(
            ArchiveMode::from_lookup(|key| values.get(key).cloned()),
            Err(ConfigError::InvalidEndpoint)
        ));

        let mut values = enabled();
        values.insert("SAKIOT_MEDIA_S3_BUCKET".to_owned(), "short".to_owned());
        assert!(matches!(
            ArchiveMode::from_lookup(|key| values.get(key).cloned()),
            Err(ConfigError::InvalidBucket)
        ));

        let mut values = enabled();
        values.insert(
            "SAKIOT_MEDIA_S3_REGION".to_owned(),
            "us-west-004".to_owned(),
        );
        values.insert(
            "SAKIOT_MEDIA_S3_ENDPOINT".to_owned(),
            "https://s3.us-west-004.backblazeb2.com".to_owned(),
        );
        assert!(matches!(
            ArchiveMode::from_lookup(|key| values.get(key).cloned()),
            Err(ConfigError::InvalidEndpoint)
        ));
    }

    #[test]
    fn enabled_uses_safe_pruning_default() {
        let values = enabled();
        let ArchiveMode::Enabled(config) =
            ArchiveMode::from_lookup(|key| values.get(key).cloned()).expect("valid config")
        else {
            panic!("archive should be enabled");
        };
        assert_eq!(config.local_retention_days, 7);
        assert_eq!(config.local_cache_max_bytes, 53_687_091_200);
        assert!(!config.local_prune_enabled);
        let debug = format!("{config:?}");
        assert!(!debug.contains("key-id"));
        assert!(!debug.contains("secret-value-should-not-appear"));
    }
}
