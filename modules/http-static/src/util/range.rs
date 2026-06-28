//! HTTP Range header parsing utilities.

use std::fmt;

/// Errors that can occur when parsing the HTTP `Range` header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeParseError {
    /// The syntax is invalid (e.g. missing `bytes=` prefix, non-numeric values).
    /// Per RFC 7233 §3.1, the server SHOULD treat this as if the header were absent.
    InvalidSyntax,
    /// The syntax is valid but the range cannot be satisfied (e.g. `bytes=100-50`).
    /// Per RFC 7233 §4.4, the server MUST respond with 416.
    Unsatisfiable,
}

impl fmt::Display for RangeParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RangeParseError::InvalidSyntax => write!(f, "invalid range syntax"),
            RangeParseError::Unsatisfiable => write!(f, "unsatisfiable range"),
        }
    }
}

impl std::error::Error for RangeParseError {}

/// Parse the HTTP Range header value.
///
/// Returns `Ok(ranges)` for valid range requests, `Err(InvalidSyntax)` for
/// syntactically invalid headers, and `Err(Unsatisfiable)` for valid syntax
/// with unsatisfiable ranges (e.g. `bytes=100-50`).
///
/// The `default_end` parameter is the last byte index (file length - 1).
///
/// Supports:
/// - Explicit ranges: `bytes=100-200`
/// - Open-ended ranges: `bytes=100-`
/// - Suffix ranges: `bytes=-500` (last 500 bytes)
/// - Multiple ranges: `bytes=100-200,300-400`
pub fn parse_range_header(
    range_str: &str,
    default_end: u64,
) -> Result<Vec<(u64, u64)>, RangeParseError> {
    let s = range_str.trim();
    let prefix = s.get(0..6).ok_or(RangeParseError::InvalidSyntax)?;
    if !prefix.eq_ignore_ascii_case("bytes=") {
        return Err(RangeParseError::InvalidSyntax);
    }
    let after = s[6..].trim();

    if after.is_empty() {
        return Err(RangeParseError::InvalidSyntax);
    }

    let mut ranges = Vec::new();
    for part in after.split(',') {
        let parts: Vec<&str> = part.trim().splitn(2, '-').collect();
        if parts.len() != 2 {
            return Err(RangeParseError::InvalidSyntax);
        }

        let a = parts[0].trim();
        let b = parts[1].trim();

        if a.is_empty() {
            let n = b
                .parse::<u64>()
                .map_err(|_| RangeParseError::InvalidSyntax)?;
            if n == 0 {
                return Err(RangeParseError::InvalidSyntax);
            }
            let file_len = default_end.saturating_add(1);
            if n >= file_len {
                ranges.push((0, default_end));
            } else {
                ranges.push((file_len - n, default_end))
            }
        } else if b.is_empty() {
            let start = a
                .parse::<u64>()
                .map_err(|_| RangeParseError::InvalidSyntax)?;
            let start = start.min(default_end);
            ranges.push((start, default_end))
        } else {
            let start = a
                .parse::<u64>()
                .map_err(|_| RangeParseError::InvalidSyntax)?;
            let end = b
                .parse::<u64>()
                .map_err(|_| RangeParseError::InvalidSyntax)?;
            if start > end {
                return Err(RangeParseError::Unsatisfiable);
            }
            ranges.push((start, end))
        }
    }

    // Coalesce overlapping ranges (permitted in RFC7233)
    ranges.sort();
    let mut result: Vec<(u64, u64)> = Vec::with_capacity(ranges.len());
    for range in &ranges {
        if let Some(last) = result.last_mut() {
            if last.1 >= range.0 {
                last.1 = last.1.max(range.1);
            } else {
                result.push(*range);
            }
        } else {
            result.push(*range);
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_explicit_range() {
        assert_eq!(
            parse_range_header("bytes=100-200", 999),
            Ok(vec![(100, 200)])
        );
    }

    #[test]
    fn parse_open_ended_range() {
        assert_eq!(parse_range_header("bytes=100-", 999), Ok(vec![(100, 999)]));
    }

    #[test]
    fn parse_suffix_range() {
        assert_eq!(parse_range_header("bytes=-500", 999), Ok(vec![(500, 999)]));
    }

    #[test]
    fn parse_suffix_range_exceeds_file() {
        assert_eq!(parse_range_header("bytes=-2000", 999), Ok(vec![(0, 999)]));
    }

    #[test]
    fn parse_suffix_range_zero() {
        assert_eq!(
            parse_range_header("bytes=-0", 999),
            Err(RangeParseError::InvalidSyntax)
        );
    }

    #[test]
    fn parse_missing_bytes_prefix() {
        assert_eq!(
            parse_range_header("100-200", 999),
            Err(RangeParseError::InvalidSyntax)
        );
    }

    #[test]
    fn parse_invalid_format() {
        assert_eq!(
            parse_range_header("bytes=100-200-300", 999),
            Err(RangeParseError::InvalidSyntax)
        );
    }

    #[test]
    fn parse_empty_range() {
        assert_eq!(
            parse_range_header("bytes=", 999),
            Err(RangeParseError::InvalidSyntax)
        );
    }

    #[test]
    fn parse_invalid_number() {
        assert_eq!(
            parse_range_header("bytes=abc-def", 999),
            Err(RangeParseError::InvalidSyntax)
        );
    }

    #[test]
    fn parse_single_dash_no_numbers() {
        assert_eq!(
            parse_range_header("bytes=-", 999),
            Err(RangeParseError::InvalidSyntax)
        );
    }

    #[test]
    fn parse_start_greater_than_end() {
        assert_eq!(
            parse_range_header("bytes=100-50", 999),
            Err(RangeParseError::Unsatisfiable)
        );
    }

    #[test]
    fn parse_multiple_ranges() {
        assert_eq!(
            parse_range_header("bytes=100-200,300-400", 500),
            Ok(vec![(100, 200), (300, 400)])
        );
    }

    #[test]
    fn parse_multiple_overlapping_ranges() {
        assert_eq!(
            parse_range_header("bytes=100-200,150-300", 500),
            Ok(vec![(100, 300)])
        );
    }
}
