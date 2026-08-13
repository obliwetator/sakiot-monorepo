//! Pure fixture selection and selector parsing.

use std::fmt::{Display, Formatter};
use std::str::FromStr;

use anyhow::{Result, bail};
use uuid::Uuid;

use crate::cli::{FixtureKind, Source};
use crate::config::DEFAULT_FIXTURE_GUILD_ID;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CountSelection {
    #[default]
    All,
    None,
    Limit(u64),
}

impl CountSelection {
    pub fn is_none(self) -> bool {
        matches!(self, Self::None | Self::Limit(0))
    }

    pub fn sql_limit(self) -> Option<u64> {
        match self {
            Self::All | Self::None => None,
            Self::Limit(value) => Some(value),
        }
    }
}

impl Display for CountSelection {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::All => f.write_str("all"),
            Self::None => f.write_str("none"),
            Self::Limit(value) => value.fmt(f),
        }
    }
}

impl FromStr for CountSelection {
    type Err = CountParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "all" => Ok(Self::All),
            "none" => Ok(Self::None),
            value => value
                .parse::<u64>()
                .map(|count| {
                    if count == 0 {
                        Self::None
                    } else {
                        Self::Limit(count)
                    }
                })
                .map_err(|_| CountParseError(value.to_string())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountParseError(String);

impl Display for CountParseError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "invalid fixture count '{}'; use all, none, or an unsigned number",
            self.0
        )
    }
}

impl std::error::Error for CountParseError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectorKind {
    FileName,
    AudioId(i64),
    Session(i64),
    Clip(Uuid),
    Stamp(i64),
    Numeric(i64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selector {
    pub input: String,
    pub kind: SelectorKind,
    pub host: Option<String>,
}

impl Selector {
    pub fn source_hint(&self) -> Option<Source> {
        self.host.as_deref().and_then(source_for_host)
    }

    pub fn value(&self) -> String {
        match &self.kind {
            SelectorKind::FileName => self
                .input
                .rsplit('/')
                .next()
                .unwrap_or(&self.input)
                .trim_end_matches(".ogg")
                .to_string(),
            SelectorKind::AudioId(id)
            | SelectorKind::Session(id)
            | SelectorKind::Stamp(id)
            | SelectorKind::Numeric(id) => id.to_string(),
            SelectorKind::Clip(id) => id.to_string(),
        }
    }
}

pub fn parse_selector(input: &str, kind: FixtureKind) -> Result<Selector> {
    if input.trim().is_empty() {
        bail!("empty fixture selector")
    }

    let (path, host) = normalize_input(input)?;
    let detected = detect_path(&path)?;

    match kind {
        FixtureKind::Auto => Ok(Selector {
            input: input.to_string(),
            kind: detected,
            host,
        }),
        FixtureKind::Recording => match detected {
            SelectorKind::Session(_) => bail!("--kind recording cannot select a session URL"),
            SelectorKind::Clip(_) => bail!("--kind recording needs an audio URL or filename"),
            SelectorKind::Stamp(_) => bail!("--kind recording cannot select a stamp"),
            SelectorKind::Numeric(id) => Ok(Selector {
                input: input.to_string(),
                kind: SelectorKind::AudioId(id),
                host,
            }),
            other => Ok(Selector {
                input: input.to_string(),
                kind: other,
                host,
            }),
        },
        FixtureKind::Session => {
            let id = numeric_path(&path).ok_or_else(|| {
                anyhow::anyhow!("--kind session needs a numeric session id or session URL")
            })?;
            if !matches!(
                detected,
                SelectorKind::Session(_) | SelectorKind::Numeric(_)
            ) {
                bail!("--kind session needs a numeric session id or session URL")
            }
            Ok(Selector {
                input: input.to_string(),
                kind: SelectorKind::Session(id),
                host,
            })
        }
        FixtureKind::Clip => {
            let id = match detected {
                SelectorKind::Clip(id) => id,
                _ => bail!("--kind clip needs a clip URL or UUID"),
            };
            Ok(Selector {
                input: input.to_string(),
                kind: SelectorKind::Clip(id),
                host,
            })
        }
        FixtureKind::Stamp => {
            let id = numeric_path(&path)
                .ok_or_else(|| anyhow::anyhow!("--kind stamp needs a numeric stamp id"))?;
            if !matches!(detected, SelectorKind::Numeric(_)) {
                bail!("--kind stamp needs a numeric stamp id")
            }
            Ok(Selector {
                input: input.to_string(),
                kind: SelectorKind::Stamp(id),
                host,
            })
        }
    }
}

fn normalize_input(input: &str) -> Result<(String, Option<String>)> {
    if input.starts_with("http://") || input.starts_with("https://") {
        let url = url::Url::parse(input)
            .map_err(|error| anyhow::anyhow!("invalid fixture URL: {error}"))?;
        let host = url.host_str().map(str::to_string);
        let path = url.path().trim_end_matches('/').to_string();
        return Ok((path, host));
    }
    if input.contains("://") {
        bail!("fixture URLs must use http:// or https://")
    }
    Ok((input.trim_end_matches('/').to_string(), None))
}

fn detect_path(path: &str) -> Result<SelectorKind> {
    let segments: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    if let Some(index) = segments.iter().position(|part| *part == "audio") {
        let tail = &segments[index + 1..];
        if tail.first().copied() == Some("session") {
            let value = tail
                .get(1)
                .ok_or_else(|| anyhow::anyhow!("audio session URL is missing its id"))?;
            return Ok(SelectorKind::Session(parse_i64(value, "session id")?));
        }
        if tail.len() == 4 {
            let file = tail[3].trim_end_matches(".ogg");
            validate_filename(file)?;
            return Ok(SelectorKind::FileName);
        }
        bail!(
            "unrecognized audio URL; expected .../audio/<channel>/<year>/<month>/<file> or .../audio/session/<id>"
        )
    }
    if let Some(index) = segments.iter().position(|part| *part == "clips") {
        let value = segments
            .get(index + 1)
            .ok_or_else(|| anyhow::anyhow!("clip URL is missing its UUID"))?;
        return Ok(SelectorKind::Clip(
            Uuid::parse_str(value).map_err(|_| anyhow::anyhow!("not a clip UUID: {value}"))?,
        ));
    }

    if segments.len() != 1 {
        bail!("unrecognized fixture selector: {path}")
    }
    let value = segments[0].trim_end_matches(".ogg");
    if value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Ok(SelectorKind::Numeric(parse_i64(value, "fixture id")?));
    }
    if value.starts_with('-') && value[1..].bytes().all(|byte| byte.is_ascii_digit()) {
        bail!("fixture ids must be unsigned")
    }
    if let Ok(id) = Uuid::parse_str(value) {
        return Ok(SelectorKind::Clip(id));
    }
    validate_filename(value)?;
    Ok(SelectorKind::FileName)
}

fn numeric_path(path: &str) -> Option<i64> {
    path.rsplit('/').next()?.parse::<i64>().ok()
}

fn parse_i64(value: &str, label: &str) -> Result<i64> {
    let id = value
        .parse::<i64>()
        .map_err(|_| anyhow::anyhow!("{label} must be an unsigned integer"))?;
    if id < 0 {
        bail!("{label} must be an unsigned integer")
    }
    Ok(id)
}

fn validate_filename(value: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        bail!("unrecognized recording filename: {value}")
    }
    Ok(())
}

pub fn source_for_host(host: &str) -> Option<Source> {
    match host.trim_start_matches("www.") {
        "patrykstyla.com" => Some(Source::Production),
        "staging.patrykstyla.com" | "debug.patrykstyla.com" => Some(Source::Staging),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumericResolution {
    Audio,
    Session,
    Ambiguous,
    Missing,
}

pub fn resolve_numeric(audio_exists: bool, session_exists: bool) -> NumericResolution {
    match (audio_exists, session_exists) {
        (true, true) => NumericResolution::Ambiguous,
        (true, false) => NumericResolution::Audio,
        (false, true) => NumericResolution::Session,
        (false, false) => NumericResolution::Missing,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BulkSelection {
    pub recordings: CountSelection,
    pub clips: CountSelection,
    pub days: Option<u64>,
    pub stamps: CountSelection,
    pub guild: Option<i64>,
    pub source: Source,
}

impl Default for BulkSelection {
    fn default() -> Self {
        Self {
            recordings: CountSelection::All,
            clips: CountSelection::All,
            days: None,
            stamps: CountSelection::All,
            guild: Some(DEFAULT_FIXTURE_GUILD_ID),
            source: Source::Staging,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_parser_accepts_all_none_zero_and_numbers() {
        assert_eq!("all".parse(), Ok(CountSelection::All));
        assert_eq!("none".parse(), Ok(CountSelection::None));
        assert_eq!("0".parse(), Ok(CountSelection::None));
        assert_eq!("12".parse(), Ok(CountSelection::Limit(12)));
        assert!("-1".parse::<CountSelection>().is_err());
    }

    #[test]
    fn bulk_defaults_to_the_live_test_guild() {
        assert_eq!(
            BulkSelection::default().guild,
            Some(DEFAULT_FIXTURE_GUILD_ID)
        );
    }

    #[test]
    fn parses_dashboard_selectors_and_infers_hosts() {
        let recording = parse_selector(
            "https://patrykstyla.com/dashboard/1/audio/2/2026/7/file.ogg",
            FixtureKind::Auto,
        )
        .unwrap();
        assert_eq!(recording.kind, SelectorKind::FileName);
        assert_eq!(recording.source_hint(), Some(Source::Production));

        let session = parse_selector(
            "https://staging.patrykstyla.com/dashboard/1/audio/session/384/",
            FixtureKind::Auto,
        )
        .unwrap();
        assert_eq!(session.kind, SelectorKind::Session(384));
        assert_eq!(session.source_hint(), Some(Source::Staging));
    }

    #[test]
    fn numeric_resolution_requires_disambiguation() {
        assert_eq!(resolve_numeric(true, true), NumericResolution::Ambiguous);
        assert_eq!(resolve_numeric(true, false), NumericResolution::Audio);
        assert_eq!(resolve_numeric(false, true), NumericResolution::Session);
    }

    #[test]
    fn path_validation_rejects_shellish_values() {
        assert!(parse_selector("../../etc/passwd", FixtureKind::Auto).is_err());
        assert!(parse_selector("1;drop table", FixtureKind::Auto).is_err());
        assert!(parse_selector("9223372036854775808", FixtureKind::Auto).is_err());
    }
}
