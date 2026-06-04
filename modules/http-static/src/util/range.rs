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
    // Tolerant parser for a single byte-range per RFC7233.
    // Policy: only single ranges are supported; multiple ranges (commas) are rejected.
    let s = range_str.trim();
    // Accept case-insensitive "bytes=" prefix
    let prefix = s.get(0..6)?;
    if !prefix.eq_ignore_ascii_case("bytes=") {
        return None;
    }
    let after = s[6..].trim();

    // Reject empty or multiple ranges
    if after.is_empty() || after.contains(',') {
        return None;
    }

    // Must contain exactly one '-' separating start and end
    if after.matches('-').count() != 1 {
        return None;
    }

    let parts: Vec<&str> = after.splitn(2, '-').collect();
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
            return Some((0, default_end));
        }
        Some((file_len - n, default_end))
    } else if b.is_empty() {
        // Open-ended: N-
        let start = a.parse::<u64>().ok()?;
        if start > default_end {
            return None;
        }
        Some((start, default_end))
    } else {
        // Explicit range: N-M
        let start = a.parse::<u64>().ok()?;
        let end = b.parse::<u64>().ok()?;
        if start > end {
            return None;
        }
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
}
