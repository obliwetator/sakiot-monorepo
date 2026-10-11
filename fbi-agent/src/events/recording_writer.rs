//! A recording's audio and waveform peaks, written in lockstep.
//!
//! Every frame the recorder writes to the Ogg file, packet or silence, also
//! goes to the recording's peaks file, so the web server reads a waveform,
//! live or finished, instead of decoding the audio. Audio matters more than
//! its waveform: when the peaks file fails it is deleted and the recording
//! goes on without one, and the web server decodes the audio for its
//! waveform once the recording has ended.

use std::io::{self, Write};
use std::path::PathBuf;

use sakiot_peaks::{FramePeak, PeaksWriter};

use super::ogg_opus_writer::OggOpusWriter;

/// Frames between peaks flushes: one 500 ms Ogg page, so a live waveform
/// trails its audio by about as much as the audio file does.
const PEAKS_FLUSH_FRAMES: u64 = 25;

pub struct RecordingWriter<W: Write> {
    audio: OggOpusWriter<W>,
    peaks: Option<Peaks<W>>,
    /// Why the peaks file was dropped, until the caller reports it.
    peaks_failure: Option<io::Error>,
}

struct Peaks<W: Write> {
    writer: PeaksWriter<W>,
    path: PathBuf,
    unflushed: u64,
}

impl<W: Write> RecordingWriter<W> {
    /// `peaks` is the opened peaks file at `peaks_path`, or why it could not
    /// be opened. Its header is flushed at once so a reader never finds a
    /// file without one.
    pub fn new(audio: OggOpusWriter<W>, peaks_path: PathBuf, peaks: io::Result<W>) -> Self {
        let mut writer = Self {
            audio,
            peaks: None,
            peaks_failure: None,
        };
        let opened = peaks.and_then(PeaksWriter::new).and_then(|mut peaks| {
            peaks.flush()?;
            Ok(peaks)
        });
        match opened {
            Ok(peaks) => {
                writer.peaks = Some(Peaks {
                    writer: peaks,
                    path: peaks_path,
                    unflushed: 0,
                });
            }
            Err(error) => writer.drop_peaks(peaks_path, error),
        }
        writer
    }

    /// One 20 ms Opus packet and the peak of its decoded audio.
    pub fn write_packet(&mut self, packet: &[u8], peak: FramePeak) -> io::Result<()> {
        self.audio.write_packet(packet)?;
        self.write_peaks(1, |peaks| peaks.push(peak));
        Ok(())
    }

    pub fn write_silence(&mut self, frames: u64) -> io::Result<()> {
        self.audio.write_silence(frames)?;
        self.write_peaks(frames, |peaks| peaks.push_silence(frames));
        Ok(())
    }

    /// Ends the Ogg stream and flushes the peaks. Only an audio failure is
    /// an error; a peaks failure is left for [`Self::take_peaks_failure`].
    pub fn finish(&mut self) -> io::Result<()> {
        let finished = self.audio.finish();
        if let Some(mut peaks) = self.peaks.take()
            && let Err(error) = peaks.writer.flush()
        {
            self.drop_peaks(peaks.path, error);
        }
        finished
    }

    /// Whether the recording still has a peaks file.
    #[cfg(test)]
    pub fn has_peaks(&self) -> bool {
        self.peaks.is_some()
    }

    /// Why the peaks file was dropped, once.
    pub fn take_peaks_failure(&mut self) -> Option<io::Error> {
        self.peaks_failure.take()
    }

    fn write_peaks(
        &mut self,
        frames: u64,
        write: impl FnOnce(&mut PeaksWriter<W>) -> io::Result<()>,
    ) {
        let Some(peaks) = self.peaks.as_mut() else {
            return;
        };
        let written = write(&mut peaks.writer).and_then(|()| {
            peaks.unflushed += frames;
            if peaks.unflushed < PEAKS_FLUSH_FRAMES {
                return Ok(());
            }
            peaks.unflushed = 0;
            peaks.writer.flush()
        });
        if let Err(error) = written
            && let Some(peaks) = self.peaks.take()
        {
            self.drop_peaks(peaks.path, error);
        }
    }

    /// Deletes a failed peaks file, so the web server sees no peaks rather
    /// than peaks that stop early.
    fn drop_peaks(&mut self, path: PathBuf, error: io::Error) {
        self.peaks = None;
        if let Err(remove_error) = std::fs::remove_file(&path)
            && remove_error.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!(
                path = %path.display(),
                "could not delete a failed waveform peaks file: {remove_error}"
            );
        }
        self.peaks_failure.get_or_insert(error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::{BufWriter, Cursor};

    fn audio() -> OggOpusWriter<Cursor<Vec<u8>>> {
        OggOpusWriter::new(Cursor::new(Vec::new()), 1, 0).unwrap()
    }

    #[test]
    fn every_frame_reaches_the_peaks_file_and_is_flushed_by_the_page() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.peaks");
        let file = File::create(&path).map(BufWriter::new);
        let ogg = File::create(dir.path().join("a.ogg")).unwrap();
        let audio = OggOpusWriter::new(BufWriter::new(ogg), 1, 0).unwrap();
        let mut writer = RecordingWriter::new(audio, path.clone(), file);
        // The header is on disk before any frame.
        assert_eq!(std::fs::read(&path).unwrap().len(), 8);

        let loud = FramePeak { min: -90, max: 100 };
        writer.write_silence(3).unwrap();
        for _ in 0..21 {
            writer.write_packet(&[0xFC, 0xFF, 0xFE], loud).unwrap();
        }
        // 24 frames: not flushed yet.
        assert_eq!(
            sakiot_peaks::parse(&std::fs::read(&path).unwrap())
                .unwrap()
                .len(),
            0
        );
        writer.write_packet(&[0xFC, 0xFF, 0xFE], loud).unwrap();
        let frames = sakiot_peaks::parse(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(frames.len(), 25);
        assert_eq!(frames[..3], [FramePeak::SILENT; 3]);
        assert!(frames[3..].iter().all(|frame| *frame == loud));

        writer.write_silence(2).unwrap();
        writer.finish().unwrap();
        assert_eq!(
            sakiot_peaks::parse(&std::fs::read(&path).unwrap())
                .unwrap()
                .len(),
            27
        );
        assert!(writer.take_peaks_failure().is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_failing_peaks_file_is_deleted_and_the_audio_goes_on() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.peaks");
        std::fs::write(&path, b"partial").unwrap();
        // /dev/full accepts the open and fails the header flush with ENOSPC.
        let full = std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/full")
            .map(|file| Box::new(file) as Box<dyn Write>);
        let audio = OggOpusWriter::new(Box::new(Vec::new()) as Box<dyn Write>, 1, 0).unwrap();
        let mut writer = RecordingWriter::new(audio, path.clone(), full);

        assert!(!writer.has_peaks());
        assert!(!path.exists());
        assert!(writer.take_peaks_failure().is_some());
        assert!(writer.take_peaks_failure().is_none(), "reported once");
        writer.write_silence(30).unwrap();
        writer.finish().unwrap();
    }

    #[test]
    fn a_peaks_file_that_could_not_be_opened_is_reported() {
        let mut writer = RecordingWriter::new(
            audio(),
            PathBuf::from("/nonexistent/a.peaks"),
            Err(io::Error::from(io::ErrorKind::PermissionDenied)),
        );
        assert_eq!(
            writer.take_peaks_failure().map(|error| error.kind()),
            Some(io::ErrorKind::PermissionDenied)
        );
        writer
            .write_packet(&[0xFC, 0xFF, 0xFE], FramePeak::SILENT)
            .unwrap();
        writer.finish().unwrap();
    }
}
