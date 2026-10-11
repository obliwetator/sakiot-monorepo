//! Bot accounts that talk in a voice channel, so a load test can measure what
//! recording costs the real bot.
//!
//! Each token is its own Discord application. Steps raise (or lower) how many
//! bots talk at once; each talking bot plays the audio file on a loop. An Ogg
//! Opus file at 48 kHz is passed through packet for packet, so what the
//! recorder receives is what a Discord client sent when it was recorded.
//!
//! Only staging builds of the bot record bots (its `loadtest` feature), and the
//! bot leaves a channel with no people in it, so a person stays in the call.
//!
//! Every step prints `window <speakers> <start> <end>` (Unix seconds) for the
//! steady part of the step, after joins have settled; `measure.sh` reads those
//! lines. Other lines start with `#`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, bail, ensure};
use clap::Parser;
use serenity::all::{ChannelId, Client, Context, EventHandler, GatewayIntents, GuildId, Ready};
use serenity::gateway::ShardManager;
use songbird::{SerenityInit, Songbird};
use tokio::sync::{Mutex, oneshot};

/// How long a bot may take to connect to the gateway.
const READY_TIMEOUT: Duration = Duration::from_secs(60);
/// Gap between two bots joining, inside Discord's voice join rate limits.
const JOIN_GAP: Duration = Duration::from_secs(2);

#[derive(Parser)]
#[command(about = "Bot accounts that talk in a voice channel for load tests")]
struct Args {
    /// File with one bot token per line; blank lines and `#` comments are
    /// skipped. Keep it outside the repository.
    #[arg(long)]
    tokens: PathBuf,
    /// Guild of the voice channel.
    #[arg(long)]
    guild: u64,
    /// Voice channel the bots talk in.
    #[arg(long)]
    channel: u64,
    /// Audio every talking bot plays on a loop. The default is
    /// speech.ogg, made by generate-speech.sh.
    #[arg(long, default_value = concat!(env!("CARGO_MANIFEST_DIR"), "/speech.ogg"))]
    audio: PathBuf,
    /// Talk without pauses, the worst case, instead of about 37% of the time.
    #[arg(long)]
    continuous: bool,
    /// How many bots talk at each step.
    #[arg(long, value_delimiter = ',', default_value = "0,1,2,4,8")]
    steps: Vec<usize>,
    /// Seconds each step lasts.
    #[arg(long, default_value_t = 300)]
    step_seconds: u64,
    /// Seconds at the start of each step left out of its window.
    #[arg(long, default_value_t = 30)]
    settle_seconds: u64,
}

/// When a bot talks: the gates of web-server/src/loadtest/media.rs, so both
/// load tests talk alike. Each talks while
/// `sin(2πt/slow) + 0.6·sin(2πt/fast + phase) > 0.35`, about 37% of the
/// time in spurts of 4–6 s on average, as real recordings do.
const TALK_PATTERNS: [(f64, f64, f64); 3] = [(23.0, 7.0, 1.3), (31.0, 5.0, 0.4), (17.0, 11.0, 2.2)];

/// Whether bot `index` talks `seconds` after it joined. Bots sharing a
/// pattern start at different points of it, so they don't talk in lockstep.
fn talks(index: usize, seconds: f64) -> bool {
    let (slow, fast, phase) = TALK_PATTERNS[index % TALK_PATTERNS.len()];
    let t = seconds + 11.0 * index as f64;
    (2.0 * std::f64::consts::PI * t / slow).sin()
        + 0.6 * (2.0 * std::f64::consts::PI * t / fast + phase).sin()
        > 0.35
}

/// One bot account: its gateway connection and voice manager.
struct Speaker {
    voice: Arc<Songbird>,
    shards: Arc<ShardManager>,
}

/// Signals once the gateway session is ready.
struct ReadySignal(Mutex<Option<oneshot::Sender<String>>>);

#[serenity::async_trait]
impl EventHandler for ReadySignal {
    async fn ready(&self, _ctx: Context, ready: Ready) {
        if let Some(sender) = self.0.lock().await.take() {
            let _ = sender.send(ready.user.name.clone());
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    ensure!(
        args.settle_seconds < args.step_seconds,
        "--settle-seconds must be shorter than --step-seconds"
    );
    ensure!(
        args.audio.is_file(),
        "no audio file at {}",
        args.audio.display()
    );
    let tokens = read_tokens(&args.tokens)?;
    let needed = args.steps.iter().copied().max().unwrap_or(0);
    if tokens.len() < needed {
        bail!(
            "the steps need {needed} bots, but {} has {} tokens",
            args.tokens.display(),
            tokens.len()
        );
    }

    let mut speakers = Vec::with_capacity(needed);
    for token in tokens.iter().take(needed) {
        speakers.push(connect(token).await?);
    }
    let guild = GuildId::new(args.guild);
    let channel = ChannelId::new(args.channel);

    let run = async {
        let mut gates = Vec::new();
        for &target in &args.steps {
            while gates.len() < target {
                let index = gates.len();
                let track = talk(&speakers[index], guild, channel, args.audio.clone()).await?;
                gates.push(tokio::spawn(gate(track, index, args.continuous)));
                tokio::time::sleep(JOIN_GAP).await;
            }
            while gates.len() > target {
                if let Some(gate) = gates.pop() {
                    gate.abort();
                }
                speakers[gates.len()].voice.remove(guild).await?;
            }
            say(&format!("# step: {target} talking"));
            tokio::time::sleep(Duration::from_secs(args.settle_seconds)).await;
            let start = chrono::Utc::now().timestamp();
            tokio::time::sleep(Duration::from_secs(args.step_seconds - args.settle_seconds)).await;
            say(&format!(
                "window {target} {start} {}",
                chrono::Utc::now().timestamp()
            ));
        }
        anyhow::Ok(())
    };
    let outcome = tokio::select! {
        outcome = run => outcome,
        _ = tokio::signal::ctrl_c() => {
            say("# interrupted");
            Ok(())
        }
    };

    for speaker in &speakers {
        let _ = speaker.voice.remove(guild).await;
        speaker.shards.shutdown_all().await;
    }
    outcome
}

fn read_tokens(path: &Path) -> anyhow::Result<Vec<String>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("could not read tokens from {}", path.display()))?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect())
}

/// Connects one bot account to the gateway and waits until it is ready.
async fn connect(token: &str) -> anyhow::Result<Speaker> {
    let voice = Songbird::serenity();
    let (ready_tx, ready_rx) = oneshot::channel();
    let mut client = Client::builder(
        token,
        GatewayIntents::GUILDS | GatewayIntents::GUILD_VOICE_STATES,
    )
    .event_handler(ReadySignal(Mutex::new(Some(ready_tx))))
    .register_songbird_with(voice.clone())
    .await
    .context("could not build a Discord client")?;
    let shards = client.shard_manager.clone();
    tokio::spawn(async move {
        if let Err(error) = client.start().await {
            say(&format!("# a bot's gateway connection ended: {error}"));
        }
    });
    let name = tokio::time::timeout(READY_TIMEOUT, ready_rx)
        .await
        .context("a bot did not connect within a minute")?
        .context("a bot's gateway connection ended before it was ready")?;
    say(&format!("# connected {name}"));
    Ok(Speaker { voice, shards })
}

/// Pauses and resumes a bot's track on its talk pattern. A paused bot sends
/// a few silent frames and then nothing, as a Discord client does.
async fn gate(track: songbird::tracks::TrackHandle, index: usize, continuous: bool) {
    if continuous {
        return;
    }
    let joined = tokio::time::Instant::now();
    let mut talking = true;
    let mut frames = tokio::time::interval(Duration::from_millis(20));
    loop {
        frames.tick().await;
        let wanted = talks(index, joined.elapsed().as_secs_f64());
        if wanted != talking {
            let changed = if wanted { track.play() } else { track.pause() };
            if changed.is_err() {
                return;
            }
            talking = wanted;
        }
    }
}

/// Joins the channel deafened (a speaker needs no audio back) and plays the
/// audio on a loop.
async fn talk(
    speaker: &Speaker,
    guild: GuildId,
    channel: ChannelId,
    audio: PathBuf,
) -> anyhow::Result<songbird::tracks::TrackHandle> {
    let call = speaker
        .voice
        .join(guild, channel)
        .await
        .context("could not join the voice channel")?;
    let mut call = call.lock().await;
    call.deafen(true).await.context("could not deafen")?;
    let track = call.play_input(songbird::input::File::new(audio).into());
    track.enable_loop().context("could not loop the audio")?;
    Ok(track)
}

/// One line to stdout; a closed stdout is not worth failing the run over.
fn say(line: &str) {
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{line}");
}

#[cfg(test)]
mod tests {
    use songbird::input::codecs::{get_codec_registry, get_probe};
    use symphonia::core::codecs::CODEC_TYPE_OPUS;

    /// A second of the recorder's stereo silence frame, as an Ogg Opus file.
    fn ogg_opus() -> Vec<u8> {
        use ogg::PacketWriteEndInfo::{EndPage, EndStream, NormalPacket};
        let mut bytes = Vec::new();
        let mut writer = ogg::PacketWriter::new(std::io::Cursor::new(&mut bytes));
        let mut head = b"OpusHead\x01\x02\0\0".to_vec();
        head.extend_from_slice(&48_000u32.to_le_bytes());
        head.extend_from_slice(&[0, 0, 0]);
        writer.write_packet(head, 1, EndPage, 0).unwrap();
        writer
            .write_packet(b"OpusTags\0\0\0\0\0\0\0\0".to_vec(), 1, EndPage, 0)
            .unwrap();
        for frame in 1..=50u64 {
            let end = if frame == 50 { EndStream } else { NormalPacket };
            writer
                .write_packet(vec![0xFC, 0xFF, 0xFE], 1, end, frame * 960)
                .unwrap();
        }
        drop(writer);
        bytes
    }

    #[test]
    fn bots_talk_about_37_percent_of_the_time_and_not_in_lockstep() {
        for index in 0..8 {
            let talking = (0..180_000)
                .filter(|frame| super::talks(index, f64::from(*frame) * 0.02))
                .count();
            let share = talking as f64 / 180_000.0;
            assert!((0.35..0.39).contains(&share), "bot {index}: {share}");
        }
        let same_pattern = (0..3_000)
            .filter(|frame| {
                let t = f64::from(*frame) * 0.02;
                super::talks(0, t) != super::talks(3, t)
            })
            .count();
        assert!(
            same_pattern > 500,
            "bots 0 and 3 share a pattern but not its timing"
        );
    }

    /// Songbird passes a lone full-volume Opus track through without
    /// re-encoding, so the recorder receives the recorded client's packets.
    #[tokio::test]
    async fn an_ogg_opus_recording_plays_as_opus_and_can_loop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("speech.ogg");
        std::fs::write(&path, ogg_opus()).unwrap();

        let input: songbird::input::Input = songbird::input::File::new(path).into();
        let input = input
            .make_playable_async(get_codec_registry(), get_probe())
            .await
            .unwrap();
        let parsed = input.parsed().unwrap();
        assert_eq!(parsed.decoder.codec_params().codec, CODEC_TYPE_OPUS);
        assert!(parsed.supports_backseek, "looping seeks back to the start");
    }
}
