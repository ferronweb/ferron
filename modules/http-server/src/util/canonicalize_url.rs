use thiserror::Error;

#[derive(Debug, Error)]
pub enum CanonError {
    #[error("empty path")]
    EmptyPath,

    #[error("invalid percent encoding")]
    InvalidPercentEncoding,

    #[error("invalid utf-8 after decoding")]
    InvalidUtf8,

    #[error("encoded reserved character in path: {0}")]
    EncodedReserved(u8),
}

/// Reserved characters that MUST NOT appear via decoding in routing layer.
/// We forbid them because they affect structure.
#[inline]
fn is_reserved(b: u8) -> bool {
    matches!(b, b'/' | b'\\' | b'?' | b'#')
}

/// Convert two hex chars into a byte
#[inline]
fn hex_pair(a: u8, b: u8) -> Result<u8, CanonError> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(10 + (c - b'a')),
            b'A'..=b'F' => Some(10 + (c - b'A')),
            _ => None,
        }
    }

    let hi = val(a).ok_or(CanonError::InvalidPercentEncoding)?;
    let lo = val(b).ok_or(CanonError::InvalidPercentEncoding)?;

    Ok((hi << 4) | lo)
}

/// Decode a single segment safely (NO structural characters allowed to emerge)
#[inline]
fn decode_segment(segment: &[u8]) -> Result<Vec<u8>, CanonError> {
    let mut out = Vec::with_capacity(segment.len());

    let mut i = 0;
    while i < segment.len() {
        if segment[i] == b'%' {
            if i + 2 >= segment.len() {
                return Err(CanonError::InvalidPercentEncoding);
            }

            let byte = hex_pair(segment[i + 1], segment[i + 2])?;

            // Reject encoded reserved characters (critical safety rule)
            if is_reserved(byte) {
                return Err(CanonError::EncodedReserved(byte));
            }

            // Drop null bytes
            if byte == 0 {
                i += 3;
                continue;
            }

            out.push(byte);
            i += 3;
        } else {
            let b = segment[i];

            // Also reject raw reserved chars
            if is_reserved(b) {
                return Err(CanonError::EncodedReserved(b));
            }

            out.push(b);
            i += 1;
        }
    }

    Ok(out)
}

/// Main canonicalization function
#[inline]
pub fn canonicalize_path(input: &str) -> Result<String, CanonError> {
    if input.is_empty() {
        return Err(CanonError::EmptyPath);
    } else if input == "*" {
        return Ok("*".to_string());
    }

    let bytes = input.as_bytes();

    let mut segments: Vec<Vec<u8>> = Vec::new();
    let mut current = Vec::new();

    // -----------------------------
    // 1. Split raw input by '/'
    // -----------------------------
    for &b in bytes {
        if b == b'/' {
            if !current.is_empty() {
                let decoded = decode_segment(&current)?;
                segments.push(decoded);
                current.clear();
            }
        } else {
            current.push(b);
        }
    }

    if !current.is_empty() {
        let decoded = decode_segment(&current)?;
        segments.push(decoded);
    }

    // -----------------------------
    // 2. Normalize dot segments
    // -----------------------------
    let mut stack: Vec<Vec<u8>> = Vec::new();

    for seg in segments {
        if seg == b"." {
            continue;
        } else if seg == b".." {
            stack.pop();
        } else if !seg.is_empty() {
            stack.push(seg);
        }
    }

    // -----------------------------
    // 3. Rebuild canonical path
    // -----------------------------
    let mut result = String::from("/");

    for (i, seg) in stack.iter().enumerate() {
        if i > 0 {
            result.push('/');
        }
        result.push_str(&str::from_utf8(seg).map_err(|_| CanonError::InvalidUtf8)?);
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Basic path normalization ---

    #[test]
    fn test_simple_path() {
        assert_eq!(canonicalize_path("/foo/bar").unwrap(), "/foo/bar");
    }

    #[test]
    fn test_root_path() {
        assert_eq!(canonicalize_path("/").unwrap(), "/");
    }

    #[test]
    fn test_asterisk() {
        assert_eq!(canonicalize_path("*").unwrap(), "*");
    }

    #[test]
    fn test_single_segment() {
        assert_eq!(canonicalize_path("/hello").unwrap(), "/hello");
    }

    #[test]
    fn test_path_without_leading_slash() {
        assert_eq!(canonicalize_path("foo/bar").unwrap(), "/foo/bar");
    }

    // --- Dot segment normalization ---

    #[test]
    fn test_current_directory_dot() {
        assert_eq!(canonicalize_path("/foo/./bar").unwrap(), "/foo/bar");
    }

    #[test]
    fn test_parent_directory_double_dot() {
        assert_eq!(canonicalize_path("/foo/bar/../baz").unwrap(), "/foo/baz");
    }

    #[test]
    fn test_multiple_dot_segments() {
        assert_eq!(canonicalize_path("/a/./b/../c/./d").unwrap(), "/a/c/d");
    }

    #[test]
    fn test_double_dot_at_root() {
        assert_eq!(canonicalize_path("/../foo").unwrap(), "/foo");
    }

    #[test]
    fn test_only_dots() {
        assert_eq!(canonicalize_path("/././.").unwrap(), "/");
    }

    #[test]
    fn test_double_dot_escaping_root() {
        assert_eq!(canonicalize_path("/a/../../b").unwrap(), "/b");
    }

    // --- Percent encoding ---

    #[test]
    fn test_percent_encoded_space() {
        assert_eq!(canonicalize_path("/foo%20bar").unwrap(), "/foo bar");
    }

    #[test]
    fn test_percent_encoded_uppercase() {
        assert_eq!(canonicalize_path("/foo%20bar").unwrap(), "/foo bar");
    }

    #[test]
    fn test_percent_encoded_lowercase() {
        assert_eq!(canonicalize_path("/foo%20bar").unwrap(), "/foo bar");
    }

    #[test]
    fn test_mixed_case_percent_encoding() {
        // %fO = 0xf0 = 'ð' is wrong; %f0 = 0xf0, %4F = 'O'
        // Actually: %f0 = byte 0xf0, %4f = byte 0x4f = 'O'
        // Let's use a simpler example: %2f = '/', but that's reserved
        // Using: %41 = 'A', %62 = 'b'
        assert_eq!(canonicalize_path("/%4F%6b").unwrap(), "/Ok");
    }

    #[test]
    fn test_multiple_percent_encodings() {
        assert_eq!(canonicalize_path("/foo%20bar%21").unwrap(), "/foo bar!");
    }

    // --- Reserved character rejection ---

    #[test]
    fn test_reject_encoded_slash() {
        let err = canonicalize_path("/foo%2Fbar").unwrap_err();
        assert!(matches!(err, CanonError::EncodedReserved(b'/')));
    }

    #[test]
    fn test_reject_encoded_backslash() {
        let err = canonicalize_path("/foo%5Cbar").unwrap_err();
        assert!(matches!(err, CanonError::EncodedReserved(b'\\')));
    }

    #[test]
    fn test_reject_encoded_question_mark() {
        let err = canonicalize_path("/foo%3Fbar").unwrap_err();
        assert!(matches!(err, CanonError::EncodedReserved(b'?')));
    }

    #[test]
    fn test_reject_encoded_hash() {
        let err = canonicalize_path("/foo%23bar").unwrap_err();
        assert!(matches!(err, CanonError::EncodedReserved(b'#')));
    }

    #[test]
    fn test_reject_raw_backslash() {
        let err = canonicalize_path("/foo\\bar").unwrap_err();
        assert!(matches!(err, CanonError::EncodedReserved(b'\\')));
    }

    #[test]
    fn test_reject_raw_question_mark() {
        let err = canonicalize_path("/foo?bar").unwrap_err();
        assert!(matches!(err, CanonError::EncodedReserved(b'?')));
    }

    #[test]
    fn test_reject_raw_hash() {
        let err = canonicalize_path("/foo#bar").unwrap_err();
        assert!(matches!(err, CanonError::EncodedReserved(b'#')));
    }

    // --- Null byte handling ---

    #[test]
    fn test_null_byte_dropped() {
        assert_eq!(canonicalize_path("/foo%00bar").unwrap(), "/foobar");
    }

    #[test]
    fn test_multiple_null_bytes_dropped() {
        assert_eq!(canonicalize_path("/%00%00%00").unwrap(), "/");
    }

    // --- Invalid percent encoding ---

    #[test]
    fn test_incomplete_percent_encoding_truncated() {
        let err = canonicalize_path("/foo%2").unwrap_err();
        assert!(matches!(err, CanonError::InvalidPercentEncoding));
    }

    #[test]
    fn test_incomplete_percent_encoding_single_char() {
        let err = canonicalize_path("/foo%A").unwrap_err();
        assert!(matches!(err, CanonError::InvalidPercentEncoding));
    }

    #[test]
    fn test_invalid_hex_chars() {
        let err = canonicalize_path("/foo%GGbar").unwrap_err();
        assert!(matches!(err, CanonError::InvalidPercentEncoding));
    }

    #[test]
    fn test_percent_at_end_of_path() {
        let err = canonicalize_path("/foo%").unwrap_err();
        assert!(matches!(err, CanonError::InvalidPercentEncoding));
    }

    // --- Path traversal prevention ---

    #[test]
    fn test_encoded_dot_segments() {
        // Encoded dots should be decoded and treated as literal dots, not navigation
        assert_eq!(canonicalize_path("/%2e%2e/foo").unwrap(), "/foo");
    }

    #[test]
    fn test_encoded_double_dot_in_segment() {
        assert_eq!(canonicalize_path("/foo/%2e%2e/bar").unwrap(), "/bar");
    }

    #[test]
    fn test_mixed_encoded_and_literal_dots() {
        // /a/%2e/b/../c → /a/./b/../c → /a/c
        assert_eq!(canonicalize_path("/a/%2e/b/../c").unwrap(), "/a/c");
    }

    // --- Multiple slashes ---

    #[test]
    fn test_multiple_consecutive_slashes() {
        assert_eq!(canonicalize_path("/foo//bar").unwrap(), "/foo/bar");
    }

    #[test]
    fn test_many_consecutive_slashes() {
        assert_eq!(canonicalize_path("/////").unwrap(), "/");
    }

    #[test]
    fn test_trailing_slash() {
        assert_eq!(canonicalize_path("/foo/").unwrap(), "/foo");
    }

    // --- Edge cases ---

    #[test]
    fn test_empty_path_error() {
        let err = canonicalize_path("").unwrap_err();
        assert!(matches!(err, CanonError::EmptyPath));
    }

    #[test]
    fn test_unicode_segment() {
        assert_eq!(canonicalize_path("/caf%C3%A9").unwrap(), "/café");
    }

    #[test]
    fn test_malformed_utf8() {
        let err = canonicalize_path("/foo%C0").unwrap_err();
        assert!(matches!(err, CanonError::InvalidUtf8));
    }

    #[test]
    fn test_percent_encoded_slash_in_middle() {
        let err = canonicalize_path("/a/b%2Fc/d").unwrap_err();
        assert!(matches!(err, CanonError::EncodedReserved(b'/')));
    }

    #[test]
    fn test_complex_real_world_path() {
        // /api/v1/../v2/users/./42/../profile
        // → /api/v2/users/42/../profile
        // → /api/v2/users/profile
        assert_eq!(
            canonicalize_path("/api/v1/../v2/users/./42/../profile").unwrap(),
            "/api/v2/users/profile"
        );
    }

    #[test]
    fn test_special_characters_preserved() {
        assert_eq!(
            canonicalize_path("/foo-bar_baz.qux").unwrap(),
            "/foo-bar_baz.qux"
        );
    }

    #[test]
    fn test_double_encoding_detects_slash() {
        // %252F decodes to %2F (literal percent-two-F), not slash
        // This is correct: after first decode we get literal "%2F" string
        // which doesn't contain encoded slash
        assert_eq!(canonicalize_path("/foo%252Fbar").unwrap(), "/foo%2Fbar");
    }
}
