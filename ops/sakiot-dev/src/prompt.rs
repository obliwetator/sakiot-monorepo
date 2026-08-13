//! Interactive prompts isolated from orchestration for deterministic tests.

#![expect(
    clippy::print_stdout,
    reason = "the terminal prompt is an intentional user-facing stdout boundary"
)]

use std::io::{self, IsTerminal, Write};

use anyhow::{Context, Result, bail};

use crate::cli::FixtureStartup;
use crate::fixtures::selection::CountSelection;

pub trait PromptIo {
    fn is_terminal(&self) -> bool;
    fn ask(&mut self, message: &str) -> Result<String>;
}

pub struct TerminalPrompt;

const DEFAULT_LATEST_RECORDINGS: u64 = 10;

impl PromptIo for TerminalPrompt {
    fn is_terminal(&self) -> bool {
        io::stdin().is_terminal() && io::stdout().is_terminal()
    }

    fn ask(&mut self, message: &str) -> Result<String> {
        print!("[dev] {message}");
        io::stdout().flush()?;
        let mut answer = String::new();
        io::stdin()
            .read_line(&mut answer)
            .context("could not read interactive input")?;
        Ok(answer.trim().to_string())
    }
}

pub fn startup_policy<P: PromptIo + ?Sized>(
    prompt: &mut P,
    requested: Option<FixtureStartup>,
) -> Result<FixtureStartup> {
    let requested = requested.unwrap_or(FixtureStartup::Prompt);
    if requested != FixtureStartup::Prompt && requested != FixtureStartup::Custom {
        return Ok(requested);
    }
    if !prompt.is_terminal() {
        bail!("non-interactive cargo dev up must specify --fixtures skip or --fixtures full")
    }
    if requested == FixtureStartup::Custom {
        return Ok(FixtureStartup::Custom);
    }
    let answer = prompt.ask("fixtures: [s]kip, [f]ull, or [c]ustom? [s] ")?;
    match answer.to_ascii_lowercase().as_str() {
        "" | "s" | "skip" => Ok(FixtureStartup::Skip),
        "f" | "full" => Ok(FixtureStartup::Full),
        "c" | "custom" => Ok(FixtureStartup::Custom),
        _ => bail!("choose skip, full, or custom"),
    }
}

pub fn custom_counts<P: PromptIo + ?Sized>(
    prompt: &mut P,
) -> Result<(CountSelection, CountSelection, CountSelection)> {
    Ok((
        ask_count(prompt, "recordings", "all mirrors staging; 0 skips")?,
        ask_count(prompt, "clips", "all mirrors staging; 0 skips")?,
        ask_count(prompt, "stamps", "all mirrors staging; 0 skips")?,
    ))
}

/// Ask for a bounded, newest-first recording selection. Interactive syncs do
/// not offer `all`: an explicit `--recordings all` is required for that.
pub fn latest_recording_count<P: PromptIo + ?Sized>(
    prompt: &mut P,
    available: u64,
) -> Result<CountSelection> {
    let default = if available == 0 {
        "none".to_string()
    } else {
        available.min(DEFAULT_LATEST_RECORDINGS).to_string()
    };
    loop {
        let answer = prompt.ask(&format!(
            "download latest recording(s) (server has {available}; enter a number or none) [{default}]: "
        ))?;
        let answer = if answer.is_empty() {
            default.as_str()
        } else {
            answer.as_str()
        };
        match answer.parse::<CountSelection>() {
            Ok(CountSelection::All) => {
                log("enter a number or none; use --recordings all explicitly for every recording")
            }
            Ok(value) => return Ok(value),
            Err(_) => log("enter an unsigned number or none"),
        }
    }
}

pub fn confirm<P: PromptIo + ?Sized>(
    prompt: &mut P,
    message: &str,
    assume_yes: bool,
) -> Result<()> {
    if assume_yes {
        return Ok(());
    }
    if !prompt.is_terminal() {
        bail!("{message}; pass --yes in non-interactive use")
    }
    let answer = prompt.ask(&format!("{message} [y/N] "))?;
    if matches!(answer.to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        bail!("aborted")
    }
}

fn ask_count<P: PromptIo + ?Sized>(
    prompt: &mut P,
    label: &str,
    note: &str,
) -> Result<CountSelection> {
    loop {
        let answer = prompt.ask(&format!("{label} ({note}) [all]: "))?;
        let answer = if answer.is_empty() {
            "all"
        } else {
            answer.as_str()
        };
        match answer.parse::<CountSelection>() {
            Ok(value) => return Ok(value),
            Err(_) => log("enter all, none, or an unsigned number"),
        }
    }
}

#[expect(
    clippy::print_stdout,
    reason = "interactive prompts are the CLI product"
)]
fn log(message: &str) {
    println!("[dev] {message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakePrompt {
        terminal: bool,
        answers: Vec<String>,
    }

    impl PromptIo for FakePrompt {
        fn is_terminal(&self) -> bool {
            self.terminal
        }

        fn ask(&mut self, _message: &str) -> Result<String> {
            Ok(self.answers.remove(0))
        }
    }

    #[test]
    fn non_tty_requires_explicit_startup_policy() {
        let mut prompt = FakePrompt {
            terminal: false,
            answers: Vec::new(),
        };
        assert!(startup_policy(&mut prompt, None).is_err());
        assert_eq!(
            startup_policy(&mut prompt, Some(FixtureStartup::Skip)).unwrap(),
            FixtureStartup::Skip
        );
    }

    #[test]
    fn prompt_defaults_to_skip_and_custom_asks_three_counts() {
        let mut prompt = FakePrompt {
            terminal: true,
            answers: vec!["".into()],
        };
        assert_eq!(
            startup_policy(&mut prompt, None).unwrap(),
            FixtureStartup::Skip
        );
        let mut prompt = FakePrompt {
            terminal: true,
            answers: vec!["all".into(), "2".into(), "none".into()],
        };
        assert_eq!(
            custom_counts(&mut prompt).unwrap(),
            (
                CountSelection::All,
                CountSelection::Limit(2),
                CountSelection::None
            )
        );
    }

    #[test]
    fn latest_recording_prompt_defaults_to_ten_and_rejects_all() {
        let mut prompt = FakePrompt {
            terminal: true,
            answers: vec!["all".into(), "3".into()],
        };
        assert_eq!(
            latest_recording_count(&mut prompt, 42).unwrap(),
            CountSelection::Limit(3)
        );

        let mut prompt = FakePrompt {
            terminal: true,
            answers: vec![String::new()],
        };
        assert_eq!(
            latest_recording_count(&mut prompt, 42).unwrap(),
            CountSelection::Limit(10)
        );
    }
}
