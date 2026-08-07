//! ETag construction, parsing, and validation utilities.

use std::time::SystemTime;

use http::header::{self, HeaderValue};

use crate::util::compression::COMP_SUFFIXES;

/// Build a header map with common response headers (ETAG or Last-Modified, Vary, Content-Type, Cache-Control).
pub fn build_response_header_map(
    etag: Option<&str>,
    last_modified: Option<&SystemTime>,
    vary: Option<HeaderValue>,
    content_type: Option<&str>,
    cache_control: Option<&str>,
) -> http::HeaderMap {
    let mut header_map = http::HeaderMap::new();

    // ETag or Last-Modified (not both - last_modified takes precedence if provided)
    if let Some(last_mod) = last_modified {
        if let Ok(val) = HeaderValue::from_str(&httpdate::fmt_http_date(*last_mod)) {
            header_map.insert(header::LAST_MODIFIED, val);
        }
    } else if let Some(e) = etag {
        if let Ok(val) = HeaderValue::from_str(&construct_etag(e, None, true)) {
            header_map.insert(header::ETAG, val);
        }
    }

    // Vary
    if let Some(v) = vary {
        header_map.insert(header::VARY, v);
    }

    // Content-Type
    if let Some(ct) = content_type {
        if let Ok(val) = HeaderValue::from_str(ct) {
            header_map.insert(header::CONTENT_TYPE, val);
        }
    }

    // Cache-Control
    if let Some(cc) = cache_control {
        if let Ok(val) = HeaderValue::from_str(cc) {
            header_map.insert(header::CACHE_CONTROL, val);
        }
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
    let (is_weak, trimmed_raw) = if weak {
        match input.strip_prefix("W/") {
            Some(s) => (true, s),
            None => (false, input),
        }
    } else {
        (false, input)
    };

    let trimmed = trimmed_raw.trim_matches('"').trim();
    if trimmed.is_empty() {
        return None;
    }

    // Detect compression suffix at the end (e.g. "abc-precompress-br" -> base="abc", suffix="br")
    for &sfx in COMP_SUFFIXES.iter() {
        let combined = format!("-{}", sfx);
        if trimmed.ends_with(&combined) {
            let mut base = trimmed[..trimmed.len() - combined.len()].to_string();
            // Remove optional "-precompress" marker if present
            if base.ends_with("-precompress") {
                base.truncate(base.len() - "-precompress".len());
            }
            // Trim any leftover '-'
            if base.ends_with('-') {
                base.pop();
            }
            return Some((base, Some(sfx.to_string()), is_weak));
        }
    }

    // No compression suffix found — return full trimmed value as ETag
    Some((trimmed.to_string(), None, is_weak))
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
    fn extract_etag_with_precompress_and_suffix() {
        let result = extract_etag_inner("\"abc-precompress-br\"", true);
        assert_eq!(
            result,
            Some(("abc".to_string(), Some("br".to_string()), false))
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
    fn roundtrip_split_etag() {
        let original = "\"abc123-deflate\"";
        if let Some((etag, suffix, weak)) = extract_etag_inner(original, true) {
            let reconstructed = construct_etag(&etag, suffix.as_deref(), weak);
            assert_eq!(reconstructed, original);
        } else {
            panic!("extract_etag_inner returned None");
        }
    }

    #[test]
    fn build_response_header_map_with_etag() {
        let result = build_response_header_map(
            Some("my-etag"),
            None,
            None,
            Some("text/html"),
            Some("no-store"),
        );

        assert_eq!(
            result.get(header::ETAG),
            Some(&HeaderValue::from_str("W/\"my-etag\"").unwrap())
        );
        assert_eq!(
            result.get(header::CONTENT_TYPE),
            Some(&HeaderValue::from_static("text/html"))
        );
        assert_eq!(
            result.get(header::CACHE_CONTROL),
            Some(&HeaderValue::from_static("no-store"))
        );
    }

    #[test]
    fn build_response_header_map_with_last_modified() {
        let now = SystemTime::now();
        let result = build_response_header_map(
            None,
            Some(&now),
            None,
            Some("text/html"),
            Some("max-age=3600"),
        );

        assert_eq!(result.get(header::LAST_MODIFIED).is_some(), true);
        assert_eq!(
            result.get(header::CONTENT_TYPE),
            Some(&HeaderValue::from_static("text/html"))
        );
    }
}
