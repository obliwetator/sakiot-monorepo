//! The public `cargo dev` grammar.

use std::fmt::{Display, Formatter};
use std::str::FromStr;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::fixtures::selection::CountSelection;

#[derive(Debug, Parser)]
#[command(
    name = "cargo dev",
    bin_name = "cargo dev",
    about = "Run the Sakiot local development environment",
    version,
    propagate_version = true,
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Start the database, optionally hydrate fixtures, and supervise both apps.
    Up(UpArgs),
    /// Manage the local PostgreSQL instance.
    Db {
        #[command(subcommand)]
        command: DbCommand,
    },
    /// Export, hydrate, and import development fixtures.
    Fixtures {
        #[command(subcommand)]
        command: FixturesCommand,
    },
    /// Remove the local database volume and manifest-tracked fixture files.
    Clean(ConfirmArgs),
    /// Generate shell completion for the cargo dev command.
    Completions(CompletionArgs),
}

#[derive(Debug, Args)]
pub struct CompletionArgs {
    /// Shell to generate completion for.
    #[arg(value_enum)]
    pub shell: clap_complete::Shell,
}

#[derive(Debug, Args)]
pub struct UpArgs {
    /// Fixture startup policy. With no value, an interactive terminal is prompted.
    #[arg(long, value_enum)]
    pub fixtures: Option<FixtureStartup>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FixtureStartup {
    Prompt,
    Skip,
    Full,
    Custom,
}

impl Display for FixtureStartup {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Prompt => "prompt",
            Self::Skip => "skip",
            Self::Full => "full",
            Self::Custom => "custom",
        })
    }
}

#[derive(Debug, Subcommand)]
pub enum DbCommand {
    /// Start Compose PostgreSQL, migrate, seed, and prepare media directories.
    Up,
    /// Stop the local Compose services while preserving the named volume.
    Down,
    /// Recreate the local volume after confirmation, then migrate and seed it.
    Reset(ConfirmArgs),
}

#[derive(Debug, Subcommand)]
pub enum FixturesCommand {
    /// Mirror a bulk selection from a deployment into local development.
    Sync(FixtureSyncArgs),
    /// Fetch one recording, session, clip, or stamp by URL, filename, UUID, or id.
    Fetch(FixtureFetchArgs),
}

#[derive(Debug, Args)]
pub struct FixtureSyncArgs {
    /// Recordings to mirror: all, none, or a newest-first count. With no
    /// category flags, the interactive command asks for this count.
    #[arg(long, value_name = "all|none|N", value_parser = parse_count)]
    pub recordings: Option<CountSelection>,
    /// Clips to mirror: all, none, or a random sample count within the guild.
    #[arg(long, value_name = "all|none|N", value_parser = parse_count)]
    pub clips: Option<CountSelection>,
    /// Mirror every recording, clip, and stamp from the last N days.
    #[arg(
        long,
        value_name = "N",
        value_parser = parse_days,
        conflicts_with_all = ["recordings", "clips", "stamps"]
    )]
    pub days: Option<u64>,
    /// Stamps to mirror: all, none, or a random sample count within the guild.
    #[arg(long, value_name = "all|none|N", value_parser = parse_count)]
    pub stamps: Option<CountSelection>,
    /// Restrict all three categories to one guild. Defaults to the live-test guild.
    #[arg(long, value_name = "ID", value_parser = parse_id)]
    pub guild: Option<i64>,
    /// Deployment to read from. Defaults to staging.
    #[arg(long, value_enum)]
    pub source: Option<Source>,
}

#[derive(Debug, Args)]
pub struct FixtureFetchArgs {
    /// URL, audio filename, clip UUID, or numeric source id.
    pub selector: String,
    /// Interpret the selector as a recording, session, clip, stamp, or auto-detect it.
    #[arg(long, value_enum, default_value_t = FixtureKind::Auto)]
    pub kind: FixtureKind,
    /// Deployment to read from. A dashboard host infers this when omitted.
    #[arg(long, value_enum)]
    pub source: Option<Source>,
    /// Import into local development or staging.
    #[arg(long, value_enum, alias = "into", default_value_t = Destination::Local)]
    pub destination: Destination,
    /// Permit a production-to-staging write without an interactive confirmation.
    #[arg(long, short = 'y')]
    pub yes: bool,
}

impl FixtureFetchArgs {
    pub fn normalized_selector(&self) -> (&str, FixtureKind) {
        (&self.selector, self.kind)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FixtureKind {
    Auto,
    Recording,
    Session,
    Clip,
    Stamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Source {
    #[value(alias = "prod")]
    Production,
    Staging,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::Staging => "staging",
        }
    }

    pub fn legacy_label(self) -> &'static str {
        match self {
            Self::Production => "prod",
            Self::Staging => "staging",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Destination {
    Local,
    Staging,
}

#[derive(Debug, Args)]
pub struct ConfirmArgs {
    /// Do not ask for confirmation.
    #[arg(long, short = 'y')]
    pub yes: bool,
}

fn parse_count(value: &str) -> Result<CountSelection, String> {
    CountSelection::from_str(value).map_err(|error| error.to_string())
}

fn parse_days(value: &str) -> Result<u64, String> {
    let days = value
        .parse::<u64>()
        .map_err(|_| "expected a positive number of days".to_string())?;
    if days == 0 {
        return Err("expected a positive number of days".into());
    }
    Ok(days)
}

fn parse_id(value: &str) -> Result<i64, String> {
    value
        .parse::<i64>()
        .map_err(|_| "expected an unsigned 64-bit integer that fits PostgreSQL bigint".into())
}

impl FromStr for Source {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "production" | "prod" => Ok(Self::Production),
            "staging" => Ok(Self::Staging),
            _ => Err("expected production or staging".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_public_command_shapes_and_counts() {
        let cli = Cli::try_parse_from([
            "cargo dev",
            "fixtures",
            "sync",
            "--recordings",
            "none",
            "--clips",
            "12",
            "--stamps",
            "all",
            "--guild",
            "42",
            "--source",
            "production",
        ])
        .unwrap();
        let Command::Fixtures {
            command: FixturesCommand::Sync(args),
        } = cli.command
        else {
            panic!("unexpected command")
        };
        assert_eq!(args.recordings, Some(CountSelection::None));
        assert_eq!(args.clips, Some(CountSelection::Limit(12)));
        assert_eq!(args.stamps, Some(CountSelection::All));
        assert_eq!(args.guild, Some(42));
        assert_eq!(args.source, Some(Source::Production));
    }

    #[test]
    fn fetch_requires_a_positional_selector() {
        assert!(Cli::try_parse_from(["cargo dev", "fixtures", "fetch"]).is_err());
        let cli = Cli::try_parse_from([
            "cargo dev",
            "fixtures",
            "fetch",
            "1204",
            "--kind",
            "stamp",
            "--destination",
            "staging",
            "--yes",
        ])
        .unwrap();
        let Command::Fixtures {
            command: FixturesCommand::Fetch(args),
        } = cli.command
        else {
            panic!("unexpected command")
        };
        assert_eq!(args.normalized_selector(), ("1204", FixtureKind::Stamp));
        assert_eq!(args.destination, Destination::Staging);
        assert!(args.yes);
    }

    #[test]
    fn fetch_rejects_invalid_kind_values() {
        assert!(
            Cli::try_parse_from(["cargo dev", "fixtures", "fetch", "384", "--kind", "other"])
                .is_err()
        );
    }

    #[test]
    fn days_requires_a_positive_number_and_is_global() {
        assert!(Cli::try_parse_from(["cargo dev", "fixtures", "sync", "--days", "0",]).is_err());
        assert!(Cli::try_parse_from(["cargo dev", "fixtures", "sync", "--days", "7",]).is_ok());
        assert!(
            Cli::try_parse_from([
                "cargo dev",
                "fixtures",
                "sync",
                "--days",
                "7",
                "--recordings",
                "20",
            ])
            .is_err()
        );
    }

    #[test]
    fn parses_completion_shell() {
        let cli = Cli::try_parse_from(["cargo dev", "completions", "zsh"]).unwrap();
        let Command::Completions(args) = cli.command else {
            panic!("unexpected command")
        };
        assert_eq!(args.shell, clap_complete::Shell::Zsh);
    }
}
