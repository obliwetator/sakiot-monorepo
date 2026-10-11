//! Peaks from an Ogg Opus stream, for audio the recorder wrote no peaks for.

use std::io::{self, BufReader, Read, Seek, SeekFrom};

use opus2::{Channels, Decoder};

use crate::{FRAME_SAMPLES, FramePeak, SAMPLE_RATE, SamplePeak};

/// Opus's 20 ms silence frame, mono and stereo. Discord clients send the first
/// when someone stops talking, and the recorder writes the second for every
/// tick a speaker is silent: over half of a long recording's packets. They
/// count as silence without being decoded, and the decoder is reset before
/// the next real packet. That drops the few milliseconds of tail a silence
/// frame decodes to after speech, which moved 83 of 29,323 one-second points
/// by at most 2/128 on an 8-hour recording.
const SILENT_PACKETS: [[u8; 3]; 2] = [[0xF8, 0xFF, 0xFE], [0xFC, 0xFF, 0xFE]];

/// The longest Opus packet, 120 ms, in mono samples.
const MAX_PACKET_SAMPLES: usize = 5_760;

const READ_BUFFER: usize = 1 << 16;

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("could not read the Ogg stream: {0}")]
    Ogg(#[from] ogg::OggReadError),
    #[error("not an Ogg Opus stream")]
    NotOpus,
    #[error("unsupported Opus channel mapping family {0}")]
    UnsupportedMapping(u8),
    #[error("Opus decoder failed: {0}")]
    Opus(#[from] opus2::Error),
}

/// Whether `packet` is a silence frame that is never decoded.
pub fn is_silent_packet(packet: &[u8]) -> bool {
    SILENT_PACKETS.iter().any(|silent| silent == packet)
}

/// Decodes an Ogg Opus stream into one peak per 20 ms frame, downmixed to
/// mono. `progress` is called with the bytes read so far.
///
/// A stream that breaks off, as a recorder that crashed leaves it, keeps the
/// frames before the break. A packet that does not decode counts as one
/// silent frame.
pub fn decode<R: Read + Seek>(
    reader: R,
    progress: impl FnMut(u64),
) -> Result<Vec<FramePeak>, DecodeError> {
    let mut packets = ogg::PacketReader::new(BufReader::with_capacity(
        READ_BUFFER,
        Progress {
            inner: reader,
            position: 0,
            report: progress,
        },
    ));
    let pre_skip = parse_head(&packets.read_packet_expected()?.data)?;
    if !packets
        .read_packet_expected()?
        .data
        .starts_with(b"OpusTags")
    {
        return Err(DecodeError::NotOpus);
    }

    let mut decoder = Decoder::new(SAMPLE_RATE, Channels::Mono)?;
    let mut pcm = vec![0i16; MAX_PACKET_SAMPLES];
    let mut frames = Frames::new(pre_skip);
    let mut end_granule = None;
    let mut reset = false;
    loop {
        let packet = match packets.read_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(ogg::OggReadError::ReadError(error))
                if error.kind() != io::ErrorKind::UnexpectedEof =>
            {
                return Err(ogg::OggReadError::ReadError(error).into());
            }
            // A torn or corrupt final page.
            Err(_) => break,
        };
        // The recorder ends a stream with an empty packet.
        if packet.data.is_empty() {
            continue;
        }
        end_granule = Some(packet.absgp_page());
        if is_silent_packet(&packet.data) {
            frames.push_silence(FRAME_SAMPLES);
            reset = true;
            continue;
        }
        if reset {
            decoder.reset_state()?;
            reset = false;
        }
        match decoder.decode(&packet.data, &mut pcm, false) {
            Ok(samples) => frames.push(pcm.get(..samples).unwrap_or_default()),
            Err(_) => {
                frames.push_silence(FRAME_SAMPLES);
                reset = true;
            }
        }
    }
    Ok(frames.finish(end_granule))
}

/// The pre-skip of an OpusHead packet (RFC 7845 §5.1).
fn parse_head(head: &[u8]) -> Result<u64, DecodeError> {
    let Some(head) = head.strip_prefix(b"OpusHead") else {
        return Err(DecodeError::NotOpus);
    };
    // Version, channel count, pre-skip (2), input rate (4), gain (2), mapping.
    let [
        _version,
        _channels,
        skip_low,
        skip_high,
        _,
        _,
        _,
        _,
        _,
        _,
        mapping,
        ..,
    ] = *head
    else {
        return Err(DecodeError::NotOpus);
    };
    if mapping != 0 {
        return Err(DecodeError::UnsupportedMapping(mapping));
    }
    Ok(u64::from(u16::from_le_bytes([skip_low, skip_high])))
}

/// Groups decoded samples into frames, after dropping the stream's pre-skip.
struct Frames {
    peaks: Vec<FramePeak>,
    current: SamplePeak,
    in_current: u32,
    pre_skip: u64,
    to_skip: u64,
}

impl Frames {
    fn new(pre_skip: u64) -> Self {
        Self {
            peaks: Vec::new(),
            current: SamplePeak::EMPTY,
            in_current: 0,
            pre_skip,
            to_skip: pre_skip,
        }
    }

    fn push(&mut self, samples: &[i16]) {
        let skipped = usize::try_from(self.to_skip)
            .unwrap_or(usize::MAX)
            .min(samples.len());
        self.to_skip -= skipped as u64;
        for &sample in samples.get(skipped..).unwrap_or_default() {
            self.current.add(i32::from(sample));
            self.in_current += 1;
            if self.in_current == FRAME_SAMPLES {
                self.end_frame();
            }
        }
    }

    fn push_silence(&mut self, samples: u32) {
        let skipped = self.to_skip.min(u64::from(samples));
        self.to_skip -= skipped;
        let mut remaining = samples - u32::try_from(skipped).unwrap_or(samples);
        while remaining > 0 {
            let taken = remaining.min(FRAME_SAMPLES - self.in_current);
            self.current.add(0);
            self.in_current += taken;
            remaining -= taken;
            if self.in_current == FRAME_SAMPLES {
                self.end_frame();
            }
        }
    }

    fn end_frame(&mut self) {
        self.peaks.push(self.current.frame_peak());
        self.current = SamplePeak::EMPTY;
        self.in_current = 0;
    }

    /// The frames, without the encoder's padding past the stream's final
    /// granule position.
    fn finish(mut self, end_granule: Option<u64>) -> Vec<FramePeak> {
        if self.in_current > 0 {
            self.end_frame();
        }
        if let Some(granule) = end_granule {
            let samples = granule.saturating_sub(self.pre_skip);
            let frames =
                usize::try_from(samples.div_ceil(u64::from(FRAME_SAMPLES))).unwrap_or(usize::MAX);
            self.peaks.truncate(frames);
        }
        self.peaks
    }
}

/// Reports how far the stream has been read.
struct Progress<R, F> {
    inner: R,
    position: u64,
    report: F,
}

impl<R: Read, F: FnMut(u64)> Read for Progress<R, F> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buffer)?;
        self.position += read as u64;
        (self.report)(self.position);
        Ok(read)
    }
}

impl<R: Seek, F> Seek for Progress<R, F> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.position = self.inner.seek(position)?;
        Ok(self.position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ogg::PacketWriteEndInfo;
    use opus2::{Application, Encoder};
    use std::io::Cursor;

    const SERIAL: u32 = 7;
    const STEREO_SILENCE: [u8; 3] = [0xFC, 0xFF, 0xFE];

    /// 20 ms of a 440 Hz tone at about half of full scale (8-bit peak ~62).
    fn tone_packets(encoder: &mut Encoder, channels: usize, frames: usize) -> Vec<Vec<u8>> {
        (0..frames)
            .map(|frame| {
                let pcm: Vec<i16> = (0..960 * channels)
                    .map(|index| {
                        let t = (frame * 960 + index / channels) as f64 / 48_000.0;
                        (16_000.0 * (2.0 * std::f64::consts::PI * 440.0 * t).sin()) as i16
                    })
                    .collect();
                let mut packet = vec![0; 1_500];
                let length = encoder.encode(&pcm, &mut packet).unwrap();
                packet.truncate(length);
                packet
            })
            .collect()
    }

    fn ogg_opus(
        channels: u8,
        pre_skip: u16,
        packets: &[Vec<u8>],
        end_granule: Option<u64>,
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut writer = ogg::PacketWriter::new(Cursor::new(&mut bytes));
        let mut head = b"OpusHead".to_vec();
        head.extend_from_slice(&[1, channels]);
        head.extend_from_slice(&pre_skip.to_le_bytes());
        head.extend_from_slice(&48_000u32.to_le_bytes());
        head.extend_from_slice(&[0, 0, 0]);
        writer
            .write_packet(head, SERIAL, PacketWriteEndInfo::EndPage, 0)
            .unwrap();
        let mut tags = b"OpusTags".to_vec();
        tags.extend_from_slice(&[0; 8]);
        writer
            .write_packet(tags, SERIAL, PacketWriteEndInfo::EndPage, 0)
            .unwrap();
        for (index, packet) in packets.iter().enumerate() {
            let last = index + 1 == packets.len();
            let granule = 960 * (index as u64 + 1);
            let (end, granule) = if last {
                (
                    PacketWriteEndInfo::EndStream,
                    end_granule.unwrap_or(granule),
                )
            } else if index % 25 == 24 {
                (PacketWriteEndInfo::EndPage, granule)
            } else {
                (PacketWriteEndInfo::NormalPacket, granule)
            };
            writer
                .write_packet(packet.clone(), SERIAL, end, granule)
                .unwrap();
        }
        drop(writer);
        bytes
    }

    fn assert_tone(peak: FramePeak) {
        assert!((50..=75).contains(&peak.max), "{peak:?}");
        assert!((-76..=-50).contains(&peak.min), "{peak:?}");
    }

    #[test]
    fn silence_frames_count_as_silence_and_tone_decodes_around_them() {
        let mut encoder = Encoder::new(48_000, Channels::Stereo, Application::Audio).unwrap();
        let mut packets = tone_packets(&mut encoder, 2, 50);
        packets.extend(std::iter::repeat_n(STEREO_SILENCE.to_vec(), 50));
        packets.extend(tone_packets(&mut encoder, 2, 25));
        let bytes = ogg_opus(2, 0, &packets, None);

        let mut reported = 0;
        let peaks = decode(Cursor::new(&bytes), |read| reported = read).unwrap();

        assert_eq!(reported, bytes.len() as u64);
        assert_eq!(peaks.len(), 125);
        peaks[10..50].iter().copied().for_each(assert_tone);
        assert!(peaks[50..100].iter().all(|peak| *peak == FramePeak::SILENT));
        peaks[105..].iter().copied().for_each(assert_tone);
    }

    #[test]
    fn drops_the_pre_skip_and_the_padding_past_the_final_granule() {
        let mut encoder = Encoder::new(48_000, Channels::Mono, Application::Audio).unwrap();
        let packets = tone_packets(&mut encoder, 1, 30);
        // 28,800 samples decoded, less a 312-sample pre-skip, is 29.7 frames.
        let whole = decode(Cursor::new(ogg_opus(1, 312, &packets, None)), |_| {}).unwrap();
        assert_eq!(whole.len(), 30);
        // Ending 1,000 samples early leaves 27,488 samples: 28.6 frames.
        let trimmed = decode(
            Cursor::new(ogg_opus(1, 312, &packets, Some(28_800 - 1_000))),
            |_| {},
        )
        .unwrap();
        assert_eq!(trimmed.len(), 29);
        assert_eq!(trimmed[..], whole[..29]);
    }

    #[test]
    fn a_torn_stream_keeps_the_frames_before_the_break() {
        let mut encoder = Encoder::new(48_000, Channels::Stereo, Application::Audio).unwrap();
        let bytes = ogg_opus(2, 0, &tone_packets(&mut encoder, 2, 100), None);
        let peaks = decode(Cursor::new(&bytes[..bytes.len() - 30]), |_| {}).unwrap();
        assert!((50..100).contains(&peaks.len()), "{}", peaks.len());
        peaks[10..].iter().copied().for_each(assert_tone);
    }

    #[test]
    fn an_undecodable_packet_is_one_silent_frame() {
        let mut encoder = Encoder::new(48_000, Channels::Stereo, Application::Audio).unwrap();
        let mut packets = tone_packets(&mut encoder, 2, 20);
        // Code 3 with a frame count of zero is not a valid Opus packet.
        packets.push(vec![0x03, 0x00]);
        packets.extend(tone_packets(&mut encoder, 2, 20));
        let peaks = decode(Cursor::new(ogg_opus(2, 0, &packets, None)), |_| {}).unwrap();
        assert_eq!(peaks.len(), 41);
        assert_eq!(peaks[20], FramePeak::SILENT);
        peaks[30..].iter().copied().for_each(assert_tone);
    }

    #[test]
    fn rejects_streams_it_cannot_read() {
        assert!(matches!(
            decode(Cursor::new(b"not an ogg stream at all".to_vec()), |_| {}),
            Err(DecodeError::Ogg(_))
        ));
        assert!(matches!(
            parse_head(b"\x01vorbis\0\0\0\0\x02\x80\xbb\0\0"),
            Err(DecodeError::NotOpus)
        ));
        assert!(matches!(
            parse_head(b"OpusHead\x01"),
            Err(DecodeError::NotOpus)
        ));
        assert!(matches!(
            parse_head(b"OpusHead\x01\x02\0\0\x80\xbb\0\0\0\0\x01"),
            Err(DecodeError::UnsupportedMapping(1))
        ));
        assert_eq!(
            parse_head(b"OpusHead\x01\x02\x38\x01\x80\xbb\0\0\0\0\0").unwrap(),
            312
        );
    }
}
