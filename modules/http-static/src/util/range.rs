//! HTTP Range header parsing utilities.

/// Parse the HTTP Range header value.
///
/// Returns `Some(vec![(start, end), ...])` for valid range requests, or `None` for invalid ones.
/// The `default_end` parameter is the last byte index (file length - 1).
///
/// Supports:
/// - Explicit ranges: `bytes=100-200`
/// - Open-ended ranges: `bytes=100-`
/// - Suffix ranges: `bytes=-500` (last 500 bytes)
/// - Multiple ranges: `bytes=100-200,300-400`
pub fn parse_range_header(range_str: &str, default_end: u64) -> Option<Vec<(u64, u64)>> {
    // Tolerant parser for a single byte-range per RFC7233.
    let s = range_str.trim();
    // Accept case-insensitive "bytes=" prefix
    let prefix = s.get(0..6)?;
    if !prefix.eq_ignore_ascii_case("bytes=") {
        return None;
    }
    let after = s[6..].trim();

    // Reject empty ranges
    if after.is_empty() {
        return None;
    }

    let mut ranges = Vec::new();
    for part in after.split(',') {
        let parts: Vec<&str> = part.trim().splitn(2, '-').collect();
        if parts.len() != 2 {
            return None;
        }

        let a = parts[0].trim();
        let b = parts[1].trim();

        if a.is_empty() {
            // Suffix range: -N (last N bytes)
            let n = b.parse::<u64>().ok()?;
            if n == 0 {
                return None;
            }
            let file_len = default_end.saturating_add(1);
            if n >= file_len {
                ranges.push((0, default_end));
            } else {
                ranges.push((file_len - n, default_end))
            }
        } else if b.is_empty() {
            // Open-ended: N-
            let start = a.parse::<u64>().ok()?;
            let start = start.min(default_end);
            ranges.push((start, default_end))
        } else {
            // Explicit range: N-M
            let start = a.parse::<u64>().ok()?;
            let end = b.parse::<u64>().ok()?;
            if start > end {
                return None;
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

    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_explicit_range() {
        assert_eq!(
            parse_range_header("bytes=100-200", 999),
            Some(vec![(100, 200)])
        );
    }

    #[test]
    fn parse_open_ended_range() {
        assert_eq!(
            parse_range_header("bytes=100-", 999),
            Some(vec![(100, 999)])
        );
    }

    #[test]
    fn parse_suffix_range() {
        assert_eq!(
            parse_range_header("bytes=-500", 999),
            Some(vec![(500, 999)])
        );
    }

    #[test]
    fn parse_suffix_range_exceeds_file() {
        assert_eq!(parse_range_header("bytes=-2000", 999), Some(vec![(0, 999)]));
    }

    #[test]
    fn parse_suffix_range_zero() {
        assert_eq!(parse_range_header("bytes=-0", 999), None);
    }

    #[test]
    fn parse_missing_bytes_prefix() {
        assert_eq!(parse_range_header("100-200", 999), None);
    }

    #[test]
    fn parse_invalid_format() {
        // The parser rejects ranges with extra dashes like "bytes=100-200-300"
        assert_eq!(parse_range_header("bytes=100-200-300", 999), None);
    }

    #[test]
    fn parse_empty_range() {
        assert_eq!(parse_range_header("bytes=", 999), None);
    }

    #[test]
    fn parse_invalid_number() {
        assert_eq!(parse_range_header("bytes=abc-def", 999), None);
    }

    #[test]
    fn parse_single_dash_no_numbers() {
        assert_eq!(parse_range_header("bytes=-", 999), None);
    }

    #[test]
    fn parse_multiple_ranges() {
        assert_eq!(
            parse_range_header("bytes=100-200,300-400", 500),
            Some(vec![(100, 200), (300, 400)])
        );
    }

    #[test]
    fn parse_multiple_overlapping_ranges() {
        assert_eq!(
            parse_range_header("bytes=100-200,150-300", 500),
            Some(vec![(100, 300)])
        );
    }
}
