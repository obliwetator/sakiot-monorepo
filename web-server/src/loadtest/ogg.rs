//! Just enough Ogg to replay a finished Opus file as if it were being
//! recorded: where each page starts and ends and the audio time it closes at.
//! Pages are copied byte for byte, so the growing file is always a valid
//! prefix of the source, as the recorder's files are.

use std::io;

const CAPTURE: &[u8; 4] = b"OggS";
const HEADER_LEN: usize = 27;
const OPUS_RATE: i64 = 48_000;

/// One page of the source file.
#[derive(Debug, Clone, Copy)]
pub(super) struct Page {
    pub offset: u64,
    pub len: u64,
    /// The audio time at the end of this page in milliseconds, or 0 for the
    /// two header pages.
    pub end_ms: i64,
}

/// The pages of a whole Ogg/Opus file. The first two are the identification
/// and comment headers.
pub(super) fn index(data: &[u8]) -> io::Result<Vec<Page>> {
    let mut pages = Vec::new();
    let mut pre_skip = 0_i64;
    let mut offset = 0_usize;
    while offset < data.len() {
        let header = data
            .get(offset..offset + HEADER_LEN)
            .ok_or_else(|| invalid("truncated page header"))?;
        if &header[..4] != CAPTURE {
            return Err(invalid("missing OggS capture pattern"));
        }
        let mut granule = [0_u8; 8];
        granule.copy_from_slice(&header[6..14]);
        let granule = i64::from_le_bytes(granule);
        let segments = usize::from(header[26]);
        let table = data
            .get(offset + HEADER_LEN..offset + HEADER_LEN + segments)
            .ok_or_else(|| invalid("truncated segment table"))?;
        let body: usize = table.iter().map(|&lace| usize::from(lace)).sum();
        let len = HEADER_LEN + segments + body;
        if offset + len > data.len() {
            return Err(invalid("truncated page body"));
        }
        if pages.is_empty() {
            // OpusHead: the pre-skip is a little-endian u16 at byte 10.
            let head = &data[offset + HEADER_LEN + segments..offset + len];
            if head.len() < 12 || &head[..8] != b"OpusHead" {
                return Err(invalid("first page is not OpusHead"));
            }
            pre_skip = i64::from(u16::from_le_bytes([head[10], head[11]]));
        }
        let end_ms = if pages.len() < 2 || granule < 0 {
            0
        } else {
            (granule - pre_skip).max(0) * 1000 / OPUS_RATE
        };
        pages.push(Page {
            offset: offset as u64,
            len: len as u64,
            end_ms,
        });
        offset += len;
    }
    if pages.len() < 3 {
        return Err(invalid("no audio pages"));
    }
    Ok(pages)
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
