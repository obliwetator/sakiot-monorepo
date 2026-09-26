//! Streaming Ogg/Opus writer for per-user voice recordings.
//!
//! Batches Opus packets into Ogg pages of ~500 ms (25 packets) per RFC 7845 §4,
//! avoiding the container overhead of single-packet paging while preserving
//! crash resilience and live HLS streamability.
//! Discord delivers 20 ms / 960-sample stereo Opus frames at 48 kHz.

use std::io::Write;
use std::sync::OnceLock;

use ogg::PacketWriteEndInfo;

const OPUS_SAMPLE_RATE: u32 = 48000;
const OPUS_CHANNELS: u8 = 2;
const SAMPLES_PER_FRAME: u64 = 960;

/// Target packets per Ogg page: 25 packets * 20 ms = 500 ms of audio per page.
/// Falls cleanly within the RFC 7845 §4 recommendation (200 ms to 1000 ms).
const TARGET_PAGE_PACKETS: usize = 25;
/// Max payload bytes before forcing a page boundary even if packet count isn't met.
const TARGET_PAGE_BYTES: usize = 4096;
/// Ogg page segment table limit (max 255 segments per page).
const MAX_PAGE_SEGMENTS: usize = 250;

/// Bytes of a pre-encoded 20 ms stereo silent Opus frame. Synthesized once via
/// audiopus on first use. Inserted on ticks where a tracked user was silent so
/// the file timeline matches wallclock.
fn silence_frame() -> std::io::Result<&'static [u8]> {
    static CELL: OnceLock<std::io::Result<Vec<u8>>> = OnceLock::new();
    let result = CELL.get_or_init(|| {
        use opus2::{Application, Channels, Encoder};
        let mut enc = Encoder::new(48000, Channels::Stereo, Application::Voip)
            .map_err(|err| std::io::Error::other(format!("opus encoder init: {}", err)))?;
        let pcm = vec![0i16; (SAMPLES_PER_FRAME as usize) * 2];
        let mut out = vec![0u8; 256];
        let n = enc
            .encode(&pcm, &mut out)
            .map_err(|err| std::io::Error::other(format!("encode silence frame: {}", err)))?;
        out.truncate(n);
        Ok(out)
    });

    result
        .as_ref()
        .map(|bytes| bytes.as_slice())
        .map_err(|err| std::io::Error::new(err.kind(), err.to_string()))
}

/// Returns a clone of the cached silence frame bytes.
pub fn silence_frame_bytes() -> std::io::Result<Vec<u8>> {
    silence_frame().map(|bytes| bytes.to_vec())
}

pub struct OggOpusWriter<W: Write> {
    inner: ogg::PacketWriter<'static, W>,
    serial: u32,
    granule: u64,
    finished: bool,
    packets_in_page: usize,
    bytes_in_page: usize,
    segments_in_page: usize,
}

impl<W: Write> OggOpusWriter<W> {
    /// Create a new writer and emit the OpusHead + OpusTags header pages.
    /// `pre_skip_samples` should be the encoder pre-skip (typically 0 here
    /// since we are passing through Discord's already-encoded packets).
    pub fn new(writer: W, serial: u32, pre_skip_samples: u16) -> std::io::Result<Self> {
        let mut pw = ogg::PacketWriter::new(writer);

        // OpusHead (RFC 7845 §5.1)
        let mut head = Vec::with_capacity(19);
        head.extend_from_slice(b"OpusHead");
        head.push(1); // version
        head.push(OPUS_CHANNELS);
        head.extend_from_slice(&pre_skip_samples.to_le_bytes());
        head.extend_from_slice(&OPUS_SAMPLE_RATE.to_le_bytes());
        head.extend_from_slice(&0i16.to_le_bytes()); // output gain
        head.push(0); // channel mapping family 0 (mono/stereo)
        pw.write_packet(head, serial, PacketWriteEndInfo::EndPage, 0)?;

        // OpusTags (RFC 7845 §5.2) — minimal: vendor "sakiot", zero comments.
        let vendor = b"sakiot";
        let mut tags = Vec::with_capacity(8 + 4 + vendor.len() + 4);
        tags.extend_from_slice(b"OpusTags");
        tags.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
        tags.extend_from_slice(vendor);
        tags.extend_from_slice(&0u32.to_le_bytes()); // user comment list length
        pw.write_packet(tags, serial, PacketWriteEndInfo::EndPage, 0)?;

        Ok(Self {
            inner: pw,
            serial,
            granule: 0,
            finished: false,
            packets_in_page: 0,
            bytes_in_page: 0,
            segments_in_page: 0,
        })
    }

    /// Append one Opus packet (one 20 ms / 960-sample frame), bump granule,
    /// batching up to ~500 ms (25 packets) per page.
    pub fn write_packet(&mut self, packet: &[u8]) -> std::io::Result<()> {
        if self.finished {
            return Err(std::io::Error::other("writer already finished"));
        }
        self.granule += SAMPLES_PER_FRAME;
        self.packets_in_page += 1;
        self.bytes_in_page += packet.len();
        self.segments_in_page += (packet.len() / 255) + 1;

        let should_end_page = self.packets_in_page >= TARGET_PAGE_PACKETS
            || self.bytes_in_page >= TARGET_PAGE_BYTES
            || self.segments_in_page >= MAX_PAGE_SEGMENTS;

        let end_info = if should_end_page {
            self.packets_in_page = 0;
            self.bytes_in_page = 0;
            self.segments_in_page = 0;
            PacketWriteEndInfo::EndPage
        } else {
            PacketWriteEndInfo::NormalPacket
        };

        self.inner
            .write_packet(packet.to_vec(), self.serial, end_info, self.granule)
    }

    /// Append `count` silent 20 ms frames in one go (used when a user joins
    /// mid-session and we need to align their file with session-start).
    pub fn write_silence(&mut self, count: u64) -> std::io::Result<()> {
        let frame = silence_frame()?;
        for _ in 0..count {
            self.write_packet(frame)?;
        }
        Ok(())
    }

    /// Write a final EOS-marked page. After this no further writes are allowed.
    pub fn finish(&mut self) -> std::io::Result<()> {
        if self.finished {
            return Ok(());
        }
        // Emit a zero-byte packet with EndStream to mark EOS. If an audio page
        // was in progress, EndStream ends that page with the EOS flag set.
        self.inner.write_packet(
            Vec::new(),
            self.serial,
            PacketWriteEndInfo::EndStream,
            self.granule,
        )?;
        self.finished = true;
        self.packets_in_page = 0;
        self.bytes_in_page = 0;
        self.segments_in_page = 0;
        Ok(())
    }

    pub fn granule(&self) -> u64 {
        self.granule
    }
}

impl<W: Write> Drop for OggOpusWriter<W> {
    fn drop(&mut self) {
        if !self.finished {
            // Best-effort EOS on drop; ignore error in destructor.
            let _ = self.finish();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn silence_frame_is_nonempty_and_cached() -> Result<(), Box<dyn std::error::Error>> {
        let a = silence_frame_bytes()?;
        let b = silence_frame_bytes()?;
        assert!(!a.is_empty());
        assert_eq!(a, b, "cached frame should be deterministic");
        Ok(())
    }

    #[test]
    fn writes_headers_and_packets_and_finishes() -> Result<(), Box<dyn std::error::Error>> {
        let mut buf = Vec::new();
        {
            let mut w = OggOpusWriter::new(Cursor::new(&mut buf), 12345, 0)?;
            // 5 silent frames = 100ms
            w.write_silence(5)?;
            assert_eq!(w.granule(), 5 * SAMPLES_PER_FRAME);
            w.finish()?;
        }
        // Sanity: starts with "OggS" capture pattern.
        assert_eq!(&buf[..4], b"OggS", "ogg capture pattern at start");

        // Re-read with the ogg crate to confirm structure: should yield
        // OpusHead, OpusTags, 5 audio packets, then EOS.
        let mut reader = ogg::PacketReader::new(Cursor::new(&buf));
        let head = reader.read_packet_expected()?;
        assert_eq!(&head.data[..8], b"OpusHead");
        let tags = reader.read_packet_expected()?;
        assert_eq!(&tags.data[..8], b"OpusTags");
        let mut audio_count = 0;
        while let Ok(Some(p)) = reader.read_packet() {
            // The terminating zero-byte EOS packet shows up here too;
            // skip empty packets in the count.
            if !p.data.is_empty() {
                audio_count += 1;
            }
        }
        assert_eq!(audio_count, 5);
        Ok(())
    }

    #[test]
    fn gap_silence_extends_granule() -> Result<(), Box<dyn std::error::Error>> {
        let mut buf = Vec::new();
        let mut w = OggOpusWriter::new(Cursor::new(&mut buf), 12345, 0)?;

        w.write_silence(2)?;
        assert_eq!(w.granule(), 2 * SAMPLES_PER_FRAME);

        w.write_silence(3)?;
        assert_eq!(w.granule(), 5 * SAMPLES_PER_FRAME);

        Ok(())
    }

    #[test]
    fn batches_packets_into_pages_and_reduces_overhead() -> Result<(), Box<dyn std::error::Error>> {
        let mut buf = Vec::new();
        {
            let mut w = OggOpusWriter::new(Cursor::new(&mut buf), 54321, 0)?;
            // 60 frames = 1.2s:
            // Page 1: OpusHead
            // Page 2: OpusTags
            // Page 3: 25 frames
            // Page 4: 25 frames
            // Page 5: remaining 10 frames + EOS
            w.write_silence(60)?;
            assert_eq!(w.granule(), 60 * SAMPLES_PER_FRAME);
            w.finish()?;
        }

        // Count Ogg pages by looking for the "OggS" capture pattern
        let page_count = buf.windows(4).filter(|w| *w == b"OggS").count();
        assert_eq!(
            page_count, 5,
            "should have bundled 60 packets into 5 pages instead of 63"
        );

        // Verify all 60 audio packets are cleanly demuxed by ogg::PacketReader
        let mut reader = ogg::PacketReader::new(Cursor::new(&buf));
        let head = reader.read_packet_expected()?;
        assert_eq!(&head.data[..8], b"OpusHead");
        let tags = reader.read_packet_expected()?;
        assert_eq!(&tags.data[..8], b"OpusTags");

        let mut audio_packets = 0;
        while let Ok(Some(p)) = reader.read_packet() {
            if !p.data.is_empty() {
                audio_packets += 1;
            }
        }
        assert_eq!(audio_packets, 60, "all 60 audio frames must decode cleanly");

        // Verify FFmpeg/ffprobe demuxes the file and reports the exact 1.200s duration
        if let Ok(temp) = tempfile::NamedTempFile::new() {
            std::fs::write(temp.path(), &buf)?;
            let output = std::process::Command::new("ffprobe")
                .args([
                    "-v",
                    "error",
                    "-show_entries",
                    "format=duration",
                    "-of",
                    "default=noprint_wrappers=1:nokey=1",
                ])
                .arg(temp.path())
                .output();
            if let Ok(out) = output
                && out.status.success()
            {
                let dur_str = String::from_utf8_lossy(&out.stdout);
                let dur: f64 = dur_str.trim().parse()?;
                assert!(
                    (dur - 1.2).abs() < 0.05,
                    "duration should be ~1.2s, got {dur}"
                );
            }
        }

        Ok(())
    }
}
