//! HTTP Range header parsing utilities.

/// Parse the HTTP Range header value.
///
/// Returns `Some((start, end))` for valid range requests, or `None` for invalid ones.
/// The `default_end` parameter is the last byte index (file length - 1).
///
/// Supports:
/// - Explicit ranges: `bytes=100-200`
/// - Open-ended ranges: `bytes=100-`
/// - Suffix ranges: `bytes=-500` (last 500 bytes)
pub fn parse_range_header(range_str: &str, default_end: u64) -> Option<(u64, u64)> {
    let range_part = range_str.strip_prefix("bytes=")?;
    let parts: Vec<&str> = range_part.split('-').take(2).collect();
    if parts.len() != 2 {
        return None;
    }
    if parts[0].is_empty() {
        // Suffix range: -N (last N bytes)
        let n = parts[1].parse::<u64>().ok()?;
        if n == 0 {
            return None;
        }
        let file_len = default_end + 1;
        if n >= file_len {
            return Some((0, default_end));
        }
        Some((file_len - n, default_end))
    } else if parts[1].is_empty() {
        // Open-ended: N-
        let start = parts[0].parse::<u64>().ok()?;
        Some((start, default_end))
    } else {
        // Explicit range: N-M
        let start = parts[0].parse::<u64>().ok()?;
        let end = parts[1].parse::<u64>().ok()?;
        Some((start, end))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_explicit_range() {
        assert_eq!(parse_range_header("bytes=100-200", 999), Some((100, 200)));
    }

    #[test]
    fn parse_open_ended_range() {
        assert_eq!(parse_range_header("bytes=100-", 999), Some((100, 999)));
    }

    #[test]
    fn parse_suffix_range() {
        assert_eq!(parse_range_header("bytes=-500", 999), Some((500, 999)));
    }

    #[test]
    fn parse_suffix_range_exceeds_file() {
        assert_eq!(parse_range_header("bytes=-2000", 999), Some((0, 999)));
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
        // The parser takes first 2 dash-separated parts, so "bytes=100-200-300" → "100"-"200"
        assert_eq!(
            parse_range_header("bytes=100-200-300", 999),
            Some((100, 200))
        );
    }

    #[test]
    fn parse_empty_range() {
        assert_eq!(parse_range_header("bytes=", 999), None);
    }

    #[test]
    fn parse_range_start_zero() {
        assert_eq!(parse_range_header("bytes=0-100", 999), Some((0, 100)));
    }

    #[test]
    fn parse_range_full_file() {
        assert_eq!(parse_range_header("bytes=0-999", 999), Some((0, 999)));
    }

    #[test]
    fn parse_range_single_byte() {
        assert_eq!(parse_range_header("bytes=50-50", 999), Some((50, 50)));
    }

    #[test]
    fn parse_range_start_beyond_end() {
        assert_eq!(
            parse_range_header("bytes=1000-1100", 999),
            Some((1000, 1100))
        );
    }

    #[test]
    fn parse_range_end_beyond_file() {
        assert_eq!(parse_range_header("bytes=900-1500", 999), Some((900, 1500)));
    }

    #[test]
    fn parse_invalid_number() {
        assert_eq!(parse_range_header("bytes=abc-def", 999), None);
    }

    #[test]
    fn parse_negative_number() {
        // Negative numbers aren't valid u64, but the parser treats `-500` as suffix range
        assert_eq!(parse_range_header("bytes=-500", 999), Some((500, 999)));
    }

    #[test]
    fn parse_single_dash_no_numbers() {
        assert_eq!(parse_range_header("bytes=-", 999), None);
    }
}
