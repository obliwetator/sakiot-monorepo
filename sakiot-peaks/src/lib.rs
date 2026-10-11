//! Waveform peaks: the lowest and highest sample of every 20 ms of audio.
//!
//! The recorder writes a peaks file beside each recording while it records,
//! from audio songbird has already decoded. Audio without one (a recording
//! made before peaks existed or whose peaks were evicted from the local
//! cache, a silence-free render, a clip) gets one from [`decode`]. Every
//! waveform the dashboard draws is condensed from peaks files, so drawing one
//! never decodes audio.
//!
//! A peaks file is the magic `SKPK` and a little-endian `u32` version, then
//! one 2-byte record per 20 ms frame of 48 kHz audio: the frame's lowest and
//! highest sample as `i8`. A reader ignores a trailing partial record, so it
//! can read a file the recorder is still writing.

use std::io::{self, Write};

pub mod decode;

/// Samples per second of the audio a frame covers.
pub const SAMPLE_RATE: u32 = 48_000;
/// Samples in one frame: 20 ms, one Discord voice packet.
pub const FRAME_SAMPLES: u32 = 960;
/// Milliseconds in one frame.
pub const FRAME_MS: i64 = 20;

const MAGIC: [u8; 4] = *b"SKPK";
const VERSION: u32 = 1;
const HEADER_LEN: usize = 8;
const RECORD_LEN: usize = 2;

fn header() -> [u8; HEADER_LEN] {
    let mut header = [0; HEADER_LEN];
    let (magic, version) = header.split_at_mut(MAGIC.len());
    magic.copy_from_slice(&MAGIC);
    version.copy_from_slice(&VERSION.to_le_bytes());
    header
}

/// The lowest and highest sample of a frame, or of a run of frames, in 8 bits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FramePeak {
    pub min: i8,
    pub max: i8,
}

impl FramePeak {
    /// A frame of digital silence.
    pub const SILENT: Self = Self { min: 0, max: 0 };

    /// The peak of interleaved 16-bit PCM, averaging the channels of each
    /// sample as `audiowaveform` did. Empty input is silent.
    pub fn of_interleaved(samples: &[i16], channels: u8) -> Self {
        let mut peak = SamplePeak::EMPTY;
        let channels = channels.max(1);
        for sample in samples.chunks_exact(usize::from(channels)) {
            let sum: i32 = sample.iter().copied().map(i32::from).sum();
            peak.add(sum / i32::from(channels));
        }
        peak.frame_peak()
    }

    /// The lower minimum and the higher maximum of both.
    #[must_use]
    pub fn merge(self, other: Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }
}

/// Running lowest and highest 16-bit sample.
#[derive(Clone, Copy, Debug)]
struct SamplePeak {
    min: i32,
    max: i32,
}

impl SamplePeak {
    const EMPTY: Self = Self {
        min: i32::MAX,
        max: i32::MIN,
    };

    fn add(&mut self, sample: i32) {
        self.min = self.min.min(sample);
        self.max = self.max.max(sample);
    }

    fn frame_peak(self) -> FramePeak {
        if self.min > self.max {
            return FramePeak::SILENT;
        }
        FramePeak {
            min: eight_bit(self.min),
            max: eight_bit(self.max),
        }
    }
}

/// A 16-bit sample in 8 bits, truncated toward zero as `audiowaveform -b 8`
/// did. Flooring (`>> 8`) instead moves most negative peaks down by one.
fn eight_bit(sample: i32) -> i8 {
    i8::try_from((sample / 256).clamp(i32::from(i8::MIN), i32::from(i8::MAX))).unwrap_or_default()
}

/// Appends frames to a peaks file.
pub struct PeaksWriter<W: Write> {
    inner: W,
}

impl<W: Write> PeaksWriter<W> {
    /// Writes the header. Readers see it once the caller flushes.
    pub fn new(mut inner: W) -> io::Result<Self> {
        inner.write_all(&header())?;
        Ok(Self { inner })
    }

    pub fn push(&mut self, peak: FramePeak) -> io::Result<()> {
        self.inner
            .write_all(&[peak.min.to_le_bytes()[0], peak.max.to_le_bytes()[0]])
    }

    pub fn push_silence(&mut self, frames: u64) -> io::Result<()> {
        for _ in 0..frames {
            self.push(FramePeak::SILENT)?;
        }
        Ok(())
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// A whole peaks file holding `frames`.
pub fn encode(frames: &[FramePeak]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(HEADER_LEN + frames.len() * RECORD_LEN);
    bytes.extend_from_slice(&header());
    for peak in frames {
        bytes.extend_from_slice(&[peak.min.to_le_bytes()[0], peak.max.to_le_bytes()[0]]);
    }
    bytes
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum FormatError {
    #[error("not a peaks file")]
    NotPeaks,
    #[error("unsupported peaks file version {0}")]
    UnsupportedVersion(u32),
}

/// The frames of a peaks file, which may still be being written.
pub fn parse(bytes: &[u8]) -> Result<Vec<FramePeak>, FormatError> {
    let Some((head, records)) = bytes.split_first_chunk::<HEADER_LEN>() else {
        // Created, but its header not yet flushed.
        return if header().starts_with(bytes) {
            Ok(Vec::new())
        } else {
            Err(FormatError::NotPeaks)
        };
    };
    let (magic, version) = head.split_at(MAGIC.len());
    if magic != MAGIC {
        return Err(FormatError::NotPeaks);
    }
    let version = u32::from_le_bytes(version.try_into().map_err(|_| FormatError::NotPeaks)?);
    if version != VERSION {
        return Err(FormatError::UnsupportedVersion(version));
    }
    Ok(records
        .chunks_exact(RECORD_LEN)
        .map(|record| FramePeak {
            min: i8::from_le_bytes([record[0]]),
            max: i8::from_le_bytes([record[1]]),
        })
        .collect())
}

/// Frames per drawn point, for about `target_points` points over
/// `frame_count` frames.
///
/// A growing (live) recording rounds up to a power of two. Its points then
/// stay where they are as it grows, and regroup only when its length doubles,
/// instead of shifting on every refresh.
pub fn frames_per_point(frame_count: usize, target_points: usize, growing: bool) -> usize {
    let frames = frame_count.div_ceil(target_points.max(1)).max(1);
    if growing {
        frames.next_power_of_two()
    } else {
        frames
    }
}

/// Merges every `frames_per_point` frames into one point. The last point may
/// cover fewer.
pub fn condense(frames: &[FramePeak], frames_per_point: usize) -> Vec<FramePeak> {
    let mut condenser = Condenser::new(frames_per_point);
    condenser.extend(frames);
    condenser.finish()
}

/// [`condense`] for frames that arrive in pieces. A run of silence costs one
/// step per point it spans, not per frame, so a long gap is cheap.
#[derive(Debug)]
pub struct Condenser {
    frames_per_point: usize,
    in_point: usize,
    point: FramePeak,
    points: Vec<FramePeak>,
}

impl Condenser {
    pub fn new(frames_per_point: usize) -> Self {
        Self {
            frames_per_point: frames_per_point.max(1),
            in_point: 0,
            point: FramePeak::SILENT,
            points: Vec::new(),
        }
    }

    pub fn extend(&mut self, frames: &[FramePeak]) {
        for &frame in frames {
            self.add(frame, 1);
        }
    }

    pub fn push_silence(&mut self, mut frames: usize) {
        while frames > 0 {
            let taken = frames.min(self.frames_per_point - self.in_point);
            self.add(FramePeak::SILENT, taken);
            frames -= taken;
        }
    }

    /// `count` frames that all peak at `peak`, none past the current point.
    fn add(&mut self, peak: FramePeak, count: usize) {
        self.point = if self.in_point == 0 {
            peak
        } else {
            self.point.merge(peak)
        };
        self.in_point += count;
        if self.in_point == self.frames_per_point {
            self.points.push(self.point);
            self.in_point = 0;
        }
    }

    pub fn finish(mut self) -> Vec<FramePeak> {
        if self.in_point > 0 {
            self.points.push(self.point);
        }
        self.points
    }
}

/// `points`, each covering `frames_per_point` frames, as an 8-bit
/// audiowaveform version 1 `.dat`: the format the dashboard parses.
pub fn encode_dat(points: &[FramePeak], frames_per_point: usize) -> Vec<u8> {
    const DAT_VERSION: u32 = 1;
    const FLAG_EIGHT_BIT: u32 = 1;
    let samples_per_point = u32::try_from(frames_per_point)
        .ok()
        .and_then(|frames| frames.checked_mul(FRAME_SAMPLES))
        .unwrap_or(u32::MAX);
    let length = u32::try_from(points.len()).unwrap_or(u32::MAX);
    let mut dat = Vec::with_capacity(20 + points.len() * 2);
    for field in [
        DAT_VERSION,
        FLAG_EIGHT_BIT,
        SAMPLE_RATE,
        samples_per_point,
        length,
    ] {
        dat.extend_from_slice(&field.to_le_bytes());
    }
    for point in points
        .iter()
        .take(usize::try_from(length).unwrap_or(usize::MAX))
    {
        dat.extend_from_slice(&[point.min.to_le_bytes()[0], point.max.to_le_bytes()[0]]);
    }
    dat
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic spread of peaks, so tests need no randomness crate.
    fn frames(count: usize) -> Vec<FramePeak> {
        let mut state = 0x2545_f491_u32;
        (0..count)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                let [a, b, ..] = state.to_le_bytes();
                let (a, b) = (i8::from_le_bytes([a]), i8::from_le_bytes([b]));
                FramePeak {
                    min: a.min(b),
                    max: a.max(b),
                }
            })
            .collect()
    }

    #[test]
    fn averages_channels_and_truncates_toward_zero() {
        // (-300 + -301) / 2 = -300, and -300 / 256 truncates to -1, not -2.
        assert_eq!(
            FramePeak::of_interleaved(&[-300, -301, 600, 0], 2),
            FramePeak { min: -1, max: 1 }
        );
        assert_eq!(
            FramePeak::of_interleaved(&[i16::MIN, i16::MAX], 1),
            FramePeak {
                min: -128,
                max: 127
            }
        );
        assert_eq!(FramePeak::of_interleaved(&[], 2), FramePeak::SILENT);
        // A ragged tail is not a sample.
        assert_eq!(
            FramePeak::of_interleaved(&[512, 512, 30_000], 2),
            FramePeak { min: 2, max: 2 }
        );
    }

    #[test]
    fn written_frames_parse_back() {
        let mut bytes = Vec::new();
        let mut writer = PeaksWriter::new(&mut bytes).unwrap();
        writer
            .push(FramePeak {
                min: -128,
                max: 127,
            })
            .unwrap();
        writer.push_silence(2).unwrap();
        writer.push(FramePeak { min: -3, max: 4 }).unwrap();
        writer.flush().unwrap();
        let expected = vec![
            FramePeak {
                min: -128,
                max: 127,
            },
            FramePeak::SILENT,
            FramePeak::SILENT,
            FramePeak { min: -3, max: 4 },
        ];
        assert_eq!(parse(&bytes).unwrap(), expected);
        assert_eq!(bytes, encode(&expected));
    }

    #[test]
    fn a_file_being_written_reads_as_far_as_it_is_complete() {
        let bytes = encode(&frames(3));
        assert_eq!(parse(&bytes[..bytes.len() - 1]).unwrap(), frames(2));
        assert_eq!(parse(&bytes[..3]).unwrap(), Vec::new());
        assert_eq!(parse(&[]).unwrap(), Vec::new());
    }

    #[test]
    fn rejects_other_files() {
        assert_eq!(parse(b"OggS\0\0\0\0\0\0"), Err(FormatError::NotPeaks));
        assert_eq!(parse(b"Ogg"), Err(FormatError::NotPeaks));
        let mut future = encode(&frames(1));
        future[4] = 2;
        assert_eq!(parse(&future), Err(FormatError::UnsupportedVersion(2)));
    }

    #[test]
    fn condensing_takes_the_extremes_of_each_group() {
        let frames = frames(1_001);
        for per_point in [1, 2, 7, 50, 1_000, 5_000] {
            let points = condense(&frames, per_point);
            assert_eq!(points.len(), frames.len().div_ceil(per_point));
            for (index, point) in points.iter().enumerate() {
                let group = &frames[index * per_point..((index + 1) * per_point).min(frames.len())];
                assert_eq!(point.min, group.iter().map(|peak| peak.min).min().unwrap());
                assert_eq!(point.max, group.iter().map(|peak| peak.max).max().unwrap());
            }
        }
        assert!(condense(&[], 4).is_empty());
    }

    #[test]
    fn condensing_in_pieces_matches_condensing_at_once() {
        let frames = frames(700);
        for per_point in [1, 3, 64, 1_000] {
            let mut condenser = Condenser::new(per_point);
            let mut expected = Vec::new();
            for (index, piece) in frames.chunks(37).enumerate() {
                condenser.extend(piece);
                expected.extend_from_slice(piece);
                let silence = index * 11;
                condenser.push_silence(silence);
                expected.extend(std::iter::repeat_n(FramePeak::SILENT, silence));
            }
            assert_eq!(condenser.finish(), condense(&expected, per_point));
        }
    }

    #[test]
    fn a_long_silence_condenses_without_visiting_each_frame() {
        // A year of 20 ms frames, as a session stuck "active" would span.
        let mut condenser = Condenser::new(1 << 20);
        condenser.push_silence(1_576_800_000);
        condenser.extend(&[FramePeak { min: -1, max: 2 }]);
        let points = condenser.finish();
        assert_eq!(points.len(), 1_504);
        assert_eq!(points[1_503], FramePeak { min: -1, max: 2 });
    }

    #[test]
    fn condensing_eight_bit_frames_matches_condensing_the_samples() {
        // Truncation is monotonic, so the 8-bit extreme of a group is the
        // 8-bit value of the group's 16-bit extreme.
        let samples: Vec<i16> = (0..9_600)
            .map(|index| i16::try_from((index * 7_919) % 65_536 - 32_768).unwrap())
            .collect();
        let per_frame: Vec<FramePeak> = samples
            .chunks(960)
            .map(|frame| FramePeak::of_interleaved(frame, 1))
            .collect();
        for (point, group) in condense(&per_frame, 3).iter().zip(samples.chunks(3 * 960)) {
            assert_eq!(*point, FramePeak::of_interleaved(group, 1));
        }
    }

    #[test]
    fn a_growing_recording_regroups_only_when_its_length_doubles() {
        assert_eq!(frames_per_point(15_000, 2_500, false), 6);
        assert_eq!(frames_per_point(15_000, 2_500, true), 8);
        assert_eq!(frames_per_point(0, 2_500, true), 1);
        let mut changes = Vec::new();
        let mut previous = frames_per_point(1, 2_500, true);
        for frame_count in 1..200_000 {
            let current = frames_per_point(frame_count, 2_500, true);
            if current != previous {
                changes.push(frame_count);
                previous = current;
            }
        }
        assert_eq!(
            changes,
            [2_501, 5_001, 10_001, 20_001, 40_001, 80_001, 160_001]
        );
    }

    #[test]
    fn dat_output_has_the_audiowaveform_version_1_layout() {
        let points = [
            FramePeak { min: -5, max: 6 },
            FramePeak {
                min: -128,
                max: 127,
            },
        ];
        let dat = encode_dat(&points, 4);
        let field =
            |index: usize| u32::from_le_bytes(dat[index * 4..index * 4 + 4].try_into().unwrap());
        assert_eq!(
            [field(0), field(1), field(2), field(3), field(4)],
            [1, 1, 48_000, 3_840, 2]
        );
        assert_eq!(&dat[20..], &[0xFB, 6, 0x80, 127]);
    }
}
