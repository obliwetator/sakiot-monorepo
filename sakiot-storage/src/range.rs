use std::str::FromStr;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ByteRange {
    pub start: u64,
    pub end: u64,
    pub total: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum RangeError {
    #[error("invalid byte range")]
    Invalid,
    #[error("byte range is not satisfiable")]
    Unsatisfiable,
}

impl ByteRange {
    pub fn parse(value: &str, total: u64) -> Result<Self, RangeError> {
        let raw = value.strip_prefix("bytes=").ok_or(RangeError::Invalid)?;
        if raw.contains(',') || raw.is_empty() {
            return Err(RangeError::Invalid);
        }
        let (start, end) = raw.split_once('-').ok_or(RangeError::Invalid)?;
        if total == 0 {
            return Err(RangeError::Unsatisfiable);
        }

        match (start.is_empty(), end.is_empty()) {
            (true, true) => Err(RangeError::Invalid),
            (true, false) => {
                let suffix = u64::from_str(end).map_err(|_| RangeError::Invalid)?;
                if suffix == 0 {
                    return Err(RangeError::Unsatisfiable);
                }
                let length = suffix.min(total);
                Ok(Self {
                    start: total - length,
                    end: total - 1,
                    total,
                })
            }
            (false, true) => {
                let start = u64::from_str(start).map_err(|_| RangeError::Invalid)?;
                if start >= total {
                    return Err(RangeError::Unsatisfiable);
                }
                Ok(Self {
                    start,
                    end: total - 1,
                    total,
                })
            }
            (false, false) => {
                let start = u64::from_str(start).map_err(|_| RangeError::Invalid)?;
                let requested_end = u64::from_str(end).map_err(|_| RangeError::Invalid)?;
                if start > requested_end || start >= total {
                    return Err(RangeError::Unsatisfiable);
                }
                Ok(Self {
                    start,
                    end: requested_end.min(total - 1),
                    total,
                })
            }
        }
    }

    pub fn content_length(self) -> u64 {
        self.end - self.start + 1
    }

    pub fn request_header(self) -> String {
        format!("bytes={}-{}", self.start, self.end)
    }

    pub fn content_range(self) -> String {
        format!("bytes {}-{}/{}", self.start, self.end, self.total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_closed_open_and_suffix_ranges() {
        assert_eq!(
            ByteRange::parse("bytes=2-5", 10),
            Ok(ByteRange {
                start: 2,
                end: 5,
                total: 10
            })
        );
        assert_eq!(
            ByteRange::parse("bytes=7-", 10).map(ByteRange::content_length),
            Ok(3)
        );
        assert_eq!(
            ByteRange::parse("bytes=-4", 10).map(|range| range.start),
            Ok(6)
        );
        assert_eq!(
            ByteRange::parse("bytes=-40", 10).map(|range| range.start),
            Ok(0)
        );
    }

    #[test]
    fn rejects_multiple_and_unsatisfiable_ranges() {
        assert_eq!(
            ByteRange::parse("bytes=0-1,4-5", 10),
            Err(RangeError::Invalid)
        );
        assert_eq!(
            ByteRange::parse("bytes=10-", 10),
            Err(RangeError::Unsatisfiable)
        );
        assert_eq!(
            ByteRange::parse("bytes=5-4", 10),
            Err(RangeError::Unsatisfiable)
        );
        assert_eq!(
            ByteRange::parse("bytes=-0", 10),
            Err(RangeError::Unsatisfiable)
        );
    }
}
