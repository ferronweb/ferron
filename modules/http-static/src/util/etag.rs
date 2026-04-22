//! ETag construction, parsing, and validation utilities.

use http::header::{self, HeaderValue};

/// Build a header map with ETag and Vary headers.
pub fn build_etag_header_map(
    etag: &str,
    vary: &Option<String>,
    content_type: Option<&str>,
    cache_control: Option<&str>,
) -> http::HeaderMap {
    let mut header_map = http::HeaderMap::new();
    header_map.insert(
        header::ETAG,
        HeaderValue::from_str(&construct_etag(etag, None, true)).expect("invalid etag header"),
    );
    if let Some(v) = vary {
        header_map.insert(
            header::VARY,
            HeaderValue::from_str(v).expect("invalid vary header"),
        );
    }
    if let Some(ct) = content_type {
        header_map.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_str(ct).expect("invalid content-type header"),
        );
    }
    if let Some(cc) = cache_control {
        header_map.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_str(cc).expect("invalid cache-control header"),
        );
    }
    header_map
}

/// Split an ETag request header into individual ETags.
pub fn split_etag_request(etag: &str) -> Vec<String> {
    let mut is_quote = false;
    let mut result = Vec::new();
    let mut current = String::new();
    let mut chars = etag.chars();

    while let Some(c) = chars.next() {
        if c == '"' {
            is_quote = !is_quote;
        } else if c == ',' && !is_quote {
            let trimmed = current.trim().to_owned();
            if !trimmed.is_empty() {
                result.push(trimmed);
            }
            current.clear();
        } else if c == '\\' && is_quote {
            if let Some(next) = chars.next() {
                current.push(next);
            }
        } else {
            current.push(c);
        }
    }
    let trimmed = current.trim().to_owned();
    if !trimmed.is_empty() {
        result.push(trimmed);
    }
    result
}

/// Extract ETag inner value, optionally handling weak ETags.
///
/// Returns `(etag_value, compression_suffix, is_weak)`.
pub fn extract_etag_inner(input: &str, weak: bool) -> Option<(String, Option<String>, bool)> {
    let (is_weak, trimmed) = if weak {
        match input.strip_prefix("W/") {
            Some(s) => (true, s),
            None => (false, input),
        }
    } else {
        (false, input)
    };
    let trimmed = trimmed.trim_matches('"');
    let mut parts = trimmed.splitn(2, '-');
    parts
        .next()
        .map(|etag| (etag.to_string(), parts.next().map(String::from), is_weak))
}

/// Construct an ETag string.
pub fn construct_etag(etag: &str, suffix: Option<&str>, weak: bool) -> String {
    let inner = match suffix {
        Some(s) => format!("{etag}-{s}"),
        None => etag.to_string(),
    };
    if weak {
        format!("W/\"{inner}\"")
    } else {
        format!("\"{inner}\"")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_etag_single() {
        let tags = split_etag_request("\"abc123\"");
        assert_eq!(tags, vec!["abc123"]);
    }

    #[test]
    fn split_etag_multiple() {
        let tags = split_etag_request("\"abc\", \"def\", \"ghi\"");
        assert_eq!(tags, vec!["abc", "def", "ghi"]);
    }

    #[test]
    fn split_etag_weak() {
        // Note: split_etag_request strips quotes from individual tokens
        let tags = split_etag_request("W/\"abc\", \"def\"");
        assert_eq!(tags, vec!["W/abc", "def"]);
    }

    #[test]
    fn split_etag_with_escaped_quotes() {
        let tags = split_etag_request("\"ab\\\"c\"");
        assert_eq!(tags, vec!["ab\"c"]);
    }

    #[test]
    fn split_etag_empty() {
        let tags = split_etag_request("");
        assert!(tags.is_empty());
    }

    #[test]
    fn split_etag_trailing_comma() {
        let tags = split_etag_request("\"abc\",");
        assert_eq!(tags, vec!["abc"]);
    }

    #[test]
    fn extract_etag_strong() {
        let result = extract_etag_inner("\"abc123\"", true);
        assert_eq!(result, Some(("abc123".to_string(), None, false)));
    }

    #[test]
    fn extract_etag_weak() {
        let result = extract_etag_inner("W/\"abc123\"", true);
        assert_eq!(result, Some(("abc123".to_string(), None, true)));
    }

    #[test]
    fn extract_etag_with_compression_suffix() {
        let result = extract_etag_inner("\"abc123-br\"", true);
        assert_eq!(
            result,
            Some(("abc123".to_string(), Some("br".to_string()), false))
        );
    }

    #[test]
    fn extract_etag_no_weak_prefix() {
        let result = extract_etag_inner("\"abc\"", false);
        assert_eq!(result, Some(("abc".to_string(), None, false)));
    }

    #[test]
    fn construct_etag_strong() {
        assert_eq!(construct_etag("abc", None, false), "\"abc\"");
    }

    #[test]
    fn construct_etag_weak() {
        assert_eq!(construct_etag("abc", None, true), "W/\"abc\"");
    }

    #[test]
    fn construct_etag_with_suffix() {
        // Note: suffix already includes the leading dash
        assert_eq!(construct_etag("abc", Some("br"), true), "W/\"abc-br\"");
    }

    #[test]
    fn construct_etag_empty_suffix() {
        assert_eq!(construct_etag("abc", Some(""), true), "W/\"abc-\"");
    }

    #[test]
    fn roundtrip_split_etag() {
        let original = "\"abc123-deflate\"";
        if let Some((etag, suffix, weak)) = extract_etag_inner(original, true) {
            let reconstructed = construct_etag(&etag, suffix.as_deref(), weak);
            assert_eq!(reconstructed, original);
        } else {
            panic!("extract_etag_inner returned None");
        }
    }
}
