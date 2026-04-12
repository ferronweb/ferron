use std::fmt;

/// Authoritative semantic path for routing, ACLs, cache keys, and scope checks.
#[derive(Debug, PartialEq)]
pub struct CanonicalizedPath {
    /// Authoritative semantic path for routing, ACLs, cache keys, and scope checks.
    ///
    /// For `"*"` input: exactly `"*"`
    /// For `"/"` paths:
    /// - Unreserved characters decoded
    /// - Reserved characters preserved as encoded
    /// - Dot-segments resolved
    /// - Root escape rejected
    /// - Trailing slash preserved
    pub routing: String,

    /// Wire-safe serialization for upstream HTTP request line or `:path`.
    ///
    /// For `"*"` input: exactly `"*"`
    /// For `"/"` paths:
    /// - Derived from the same canonical segment structure as `routing`
    /// - Reserved characters remain encoded
    /// - Hex digits uppercased for determinism
    /// - Trailing slash preserved
    ///
    /// Do not parse this value for security decisions.
    pub forwarding: String,

    /// Untouched client input for audit logging, HMAC verification, and debugging.
    /// Never use for routing, ACLs, or cache keys.
    pub original: String,
}

/// Errors that can occur during path canonicalization.
#[derive(Debug, Clone, PartialEq)]
pub enum CanonicalizationError {
    /// Input does not start with `/` or contains invalid characters.
    MalformedPath,
    /// Malformed percent-encoding such as `%`, `%G`, `%2`, or incomplete triplets.
    MalformedPercent,
    /// Dot-segment resolution would escape above root (e.g., `/../admin`).
    RootEscape,
    /// Excessive nested encoding such as `%25xx` that would create a second decoding layer.
    ExcessiveEncoding,
}

impl fmt::Display for CanonicalizationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CanonicalizationError::MalformedPath => write!(f, "malformed request path"),
            CanonicalizationError::MalformedPercent => write!(f, "malformed percent-encoding"),
            CanonicalizationError::RootEscape => write!(f, "path escapes above root"),
            CanonicalizationError::ExcessiveEncoding => write!(f, "excessive nested encoding"),
        }
    }
}

impl std::error::Error for CanonicalizationError {}

/// Returns true if the byte is an unreserved character per RFC 3986.
fn is_unreserved(b: u8) -> bool {
    matches!(b,
        b'A'..=b'Z'
        | b'a'..=b'z'
        | b'0'..=b'9'
        | b'-'
        | b'.'
        | b'_'
        | b'~'
    )
}

fn is_hex_digit(b: u8) -> bool {
    b.is_ascii_hexdigit()
}

fn hex_value(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => unreachable!(),
    }
}

/// Validates that a raw segment has well-formed percent-encoding and
/// rejects excessive nested encoding (`%25xx` patterns).
fn validate_segment_encoding(segment: &str) -> Result<(), CanonicalizationError> {
    let bytes = segment.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            // Need at least 2 more bytes for a valid triplet
            if i + 2 >= bytes.len() {
                return Err(CanonicalizationError::MalformedPercent);
            }
            let h1 = bytes[i + 1];
            let h2 = bytes[i + 2];
            if !is_hex_digit(h1) || !is_hex_digit(h2) {
                return Err(CanonicalizationError::MalformedPercent);
            }
            // Check for excessive encoding: %25 followed by two hex digits
            // This would decode to %xx on a second pass, which is dangerous.
            if h1 == b'2' && h2 == b'5' && i + 4 < bytes.len() {
                let h3 = bytes[i + 3];
                let h4 = bytes[i + 4];
                if is_hex_digit(h3) && is_hex_digit(h4) {
                    return Err(CanonicalizationError::ExcessiveEncoding);
                }
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    Ok(())
}

/// Decodes a single segment, returning (routing_decoded, forwarding_encoded).
///
/// - Unreserved characters are decoded in the routing view.
/// - Reserved characters remain encoded in both views.
/// - Hex digits are uppercased in the forwarding view.
fn decode_segment(segment: &str) -> (String, String) {
    if !segment.contains('%') {
        return (segment.to_owned(), segment.to_owned());
    }

    let bytes = segment.as_bytes();
    let mut routing = String::with_capacity(segment.len());
    let mut forwarding = String::with_capacity(segment.len());

    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let h1 = bytes[i + 1];
            let h2 = bytes[i + 2];
            let value = (hex_value(h1) << 4) | hex_value(h2);

            if is_unreserved(value) {
                // Decode unreserved characters for routing
                routing.push(value as char);
                // Forwarding keeps the encoding but uppercased
                forwarding.push('%');
                forwarding.push(h1.to_ascii_uppercase() as char);
                forwarding.push(h2.to_ascii_uppercase() as char);
            } else {
                // Reserved characters stay encoded in both views
                routing.push('%');
                routing.push(h1.to_ascii_uppercase() as char);
                routing.push(h2.to_ascii_uppercase() as char);

                forwarding.push('%');
                forwarding.push(h1.to_ascii_uppercase() as char);
                forwarding.push(h2.to_ascii_uppercase() as char);
            }
            i += 3;
        } else {
            let c = bytes[i] as char;
            routing.push(c);
            forwarding.push(c);
            i += 1;
        }
    }

    (routing, forwarding)
}

/// Decode only routing view of a single segment (no forwarding allocation).
fn decode_segment_routing(segment: &str) -> String {
    if !segment.contains('%') {
        return segment.to_owned();
    }

    let bytes = segment.as_bytes();
    let mut routing = String::with_capacity(segment.len());

    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let h1 = bytes[i + 1];
            let h2 = bytes[i + 2];
            let value = (hex_value(h1) << 4) | hex_value(h2);

            if is_unreserved(value) {
                routing.push(value as char);
            } else {
                routing.push('%');
                routing.push(h1.to_ascii_uppercase() as char);
                routing.push(h2.to_ascii_uppercase() as char);
            }
            i += 3;
        } else {
            routing.push(bytes[i] as char);
            i += 1;
        }
    }

    routing
}

/// Resolves dot-segments from a list of decoded segments using a stack.
///
/// - `.` and empty segments are skipped.
/// - `..` pops the previous segment.
/// - If `..` would pop above root, returns `RootEscape`.
fn resolve_dot_segments(segments: &[String]) -> Result<Vec<String>, CanonicalizationError> {
    let mut stack: Vec<String> = Vec::new();

    for segment in segments {
        if segment == "." || segment.is_empty() {
            // Skip current-dir and empty segments
            continue;
        } else if segment == ".." {
            // Pop the previous segment; reject if at root
            if stack.is_empty() {
                return Err(CanonicalizationError::RootEscape);
            }
            stack.pop();
        } else {
            stack.push(segment.clone());
        }
    }

    Ok(stack)
}

/// Canonicalizes a raw HTTP request target path.
///
/// Supports:
/// - Absolute paths beginning with `/`
/// - The special asterisk form `*` used for server-wide OPTIONS requests
///
/// See `URL_CANONICALIZE_SPEC.md` for the full specification.
pub fn canonicalize_path_routing(
    raw_path: &str,
) -> Result<(String, String), CanonicalizationError> {
    // Step 0: Asterisk short-circuit
    if raw_path == "*" {
        return Ok(("*".to_owned(), "*".to_owned()));
    }

    // Step 1: Syntax validation
    if !raw_path.starts_with('/') {
        return Err(CanonicalizationError::MalformedPath);
    }

    // Reject null bytes and control characters
    for &b in raw_path.as_bytes() {
        if b == 0 || b < 0x20 || b == 0x7F {
            return Err(CanonicalizationError::MalformedPath);
        }
    }

    // Step 2: Structural segmentation (split on literal '/')
    let segments: Vec<&str> = raw_path.split('/').collect();

    // Track trailing slash: present if path ends with `/` and is not just `/`
    let trailing_slash = raw_path.ends_with('/') && raw_path != "/";

    // Step 3: Per-segment validation and decoding (routing only)
    let mut routing_segments: Vec<String> = Vec::with_capacity(segments.len());

    for (idx, segment) in segments.iter().enumerate() {
        // The first segment is always empty (before the leading `/`)
        if idx == 0 {
            // Validate encoding even for the empty first segment
            validate_segment_encoding(segment)?;
            routing_segments.push(String::new());
            continue;
        }

        // Validate percent-encoding
        validate_segment_encoding(segment)?;

        // Decode segment for routing view only
        let decoded = decode_segment_routing(segment);
        routing_segments.push(decoded);
    }

    // Step 4: Dot-segment resolution (skip the first empty segment representing root)
    let dot_segments = &routing_segments[1..];
    let resolved = resolve_dot_segments(dot_segments)?;

    // Step 5: Canonical reconstruction (routing only)
    let mut routing = String::with_capacity(raw_path.len());
    routing.push('/');
    for (i, seg) in resolved.iter().enumerate() {
        if i > 0 {
            routing.push('/');
        }
        routing.push_str(seg);
    }
    if trailing_slash && routing != "/" {
        routing.push('/');
    }

    Ok((routing, raw_path.to_owned()))
}

/// Canonicalizes a raw HTTP request target path.
///
/// Supports:
/// - Absolute paths beginning with `/`
/// - The special asterisk form `*` used for server-wide OPTIONS requests
///
/// See `URL_CANONICALIZE_SPEC.md` for the full specification.
pub fn canonicalize_path(raw_path: &str) -> Result<CanonicalizedPath, CanonicalizationError> {
    // Step 0: Asterisk short-circuit
    if raw_path == "*" {
        return Ok(CanonicalizedPath {
            routing: "*".to_owned(),
            forwarding: "*".to_owned(),
            original: raw_path.to_owned(),
        });
    }

    // Step 1: Syntax validation
    // Must start with `/`
    if !raw_path.starts_with('/') {
        return Err(CanonicalizationError::MalformedPath);
    }

    // Reject null bytes and control characters
    for &b in raw_path.as_bytes() {
        if b == 0 || b < 0x20 || b == 0x7F {
            return Err(CanonicalizationError::MalformedPath);
        }
    }

    // Step 2: Structural segmentation
    // Split on literal `/` only
    let segments: Vec<&str> = raw_path.split('/').collect();

    // Track trailing slash: present if path ends with `/` and is not just `"/"`
    let trailing_slash = raw_path.ends_with('/') && raw_path != "/";

    // Step 3: Per-segment validation and decoding
    let mut routing_segments: Vec<String> = Vec::with_capacity(segments.len());
    let mut forwarding_segments: Vec<String> = Vec::with_capacity(segments.len());

    for (idx, segment) in segments.iter().enumerate() {
        // The first segment is always empty (before the leading `/`)
        if idx == 0 {
            // Validate encoding even for the empty first segment
            validate_segment_encoding(segment)?;
            routing_segments.push(String::new());
            forwarding_segments.push(String::new());
            continue;
        }

        // Validate percent-encoding
        validate_segment_encoding(segment)?;

        // Decode the segment
        let (decoded, encoded) = decode_segment(segment);
        routing_segments.push(decoded);
        forwarding_segments.push(encoded);
    }

    // Step 4: Dot-segment resolution (skip the first empty segment representing root)
    // We resolve dots on segments[1..] and keep the root empty segment
    let dot_segments = &routing_segments[1..];
    let resolved = resolve_dot_segments(dot_segments)?;

    // Step 5: Canonical reconstruction
    // routing: join resolved decoded segments, prepend `/`, add trailing slash if flagged
    let mut routing = String::with_capacity(raw_path.len());
    routing.push('/');
    for (i, seg) in resolved.iter().enumerate() {
        if i > 0 {
            routing.push('/');
        }
        routing.push_str(seg);
    }
    if trailing_slash && routing != "/" {
        routing.push('/');
    }

    // forwarding: same segment structure but with encoded segments
    // We need to resolve dots on the encoded segments too, but they have the same structure
    // (same number and position of segments after dot resolution)
    let encoded_dot_segments = &forwarding_segments[1..];
    let resolved_encoded = resolve_dot_segments(encoded_dot_segments)?;

    let mut forwarding = String::with_capacity(raw_path.len());
    forwarding.push('/');
    for (i, seg) in resolved_encoded.iter().enumerate() {
        if i > 0 {
            forwarding.push('/');
        }
        forwarding.push_str(seg);
    }
    if trailing_slash && forwarding != "/" {
        forwarding.push('/');
    }

    Ok(CanonicalizedPath {
        routing,
        forwarding,
        original: raw_path.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_asterisk_form() {
        let result = canonicalize_path("*").unwrap();
        assert_eq!(result.routing, "*");
        assert_eq!(result.forwarding, "*");
        assert_eq!(result.original, "*");
    }

    #[test]
    fn test_double_asterisk_rejected() {
        assert!(matches!(
            canonicalize_path("**"),
            Err(CanonicalizationError::MalformedPath)
        ));
    }

    #[test]
    fn test_asterisk_with_space_rejected() {
        assert!(matches!(
            canonicalize_path("* "),
            Err(CanonicalizationError::MalformedPath)
        ));
        assert!(matches!(
            canonicalize_path(" *"),
            Err(CanonicalizationError::MalformedPath)
        ));
    }

    #[test]
    fn test_root_path() {
        let result = canonicalize_path("/").unwrap();
        assert_eq!(result.routing, "/");
        assert_eq!(result.forwarding, "/");
        assert_eq!(result.original, "/");
    }

    #[test]
    fn test_simple_path() {
        let result = canonicalize_path("/api/v2").unwrap();
        assert_eq!(result.routing, "/api/v2");
        assert_eq!(result.forwarding, "/api/v2");
        assert_eq!(result.original, "/api/v2");
    }

    #[test]
    fn test_trailing_slash_preserved() {
        let result = canonicalize_path("/users/").unwrap();
        assert_eq!(result.routing, "/users/");
        assert_eq!(result.forwarding, "/users/");
        assert_eq!(result.original, "/users/");
    }

    #[test]
    fn test_percent_encoded_reserved() {
        let result = canonicalize_path("/api%2Fv2").unwrap();
        assert_eq!(result.routing, "/api%2Fv2");
        assert_eq!(result.forwarding, "/api%2Fv2");
    }

    #[test]
    fn test_percent_encoded_unreserved_decoded() {
        let result = canonicalize_path("/%41pi").unwrap();
        assert_eq!(result.routing, "/Api");
        assert_eq!(result.forwarding, "/%41pi");
    }

    #[test]
    fn test_hex_uppercased() {
        let result = canonicalize_path("/%2f").unwrap();
        assert_eq!(result.routing, "/%2F");
        assert_eq!(result.forwarding, "/%2F");
    }

    #[test]
    fn test_malformed_percent_incomplete() {
        assert!(matches!(
            canonicalize_path("/%2"),
            Err(CanonicalizationError::MalformedPercent)
        ));
    }

    #[test]
    fn test_malformed_percent_no_hex() {
        assert!(matches!(
            canonicalize_path("/%GH"),
            Err(CanonicalizationError::MalformedPercent)
        ));
    }

    #[test]
    fn test_malformed_percent_bare() {
        assert!(matches!(
            canonicalize_path("/path%"),
            Err(CanonicalizationError::MalformedPercent)
        ));
    }

    #[test]
    fn test_excessive_encoding_rejected() {
        assert!(matches!(
            canonicalize_path("/%252Ftest"),
            Err(CanonicalizationError::ExcessiveEncoding)
        ));
    }

    #[test]
    fn test_double_percent_25() {
        assert!(matches!(
            canonicalize_path("/%25AB"),
            Err(CanonicalizationError::ExcessiveEncoding)
        ));
    }

    #[test]
    fn test_dot_segment_current_dir() {
        let result = canonicalize_path("/a/./b").unwrap();
        assert_eq!(result.routing, "/a/b");
        assert_eq!(result.forwarding, "/a/b");
    }

    #[test]
    fn test_dot_segment_parent_dir() {
        let result = canonicalize_path("/a/../b").unwrap();
        assert_eq!(result.routing, "/b");
        assert_eq!(result.forwarding, "/b");
    }

    #[test]
    fn test_root_escape_rejected() {
        assert!(matches!(
            canonicalize_path("/../admin"),
            Err(CanonicalizationError::RootEscape)
        ));
    }

    #[test]
    fn test_multiple_dot_segments() {
        let result = canonicalize_path("/a/b/../../c").unwrap();
        assert_eq!(result.routing, "/c");
        assert_eq!(result.forwarding, "/c");
    }

    #[test]
    fn test_empty_interior_segments_collapsed() {
        let result = canonicalize_path("/api//v2").unwrap();
        assert_eq!(result.routing, "/api/v2");
        assert_eq!(result.forwarding, "/api/v2");
    }

    #[test]
    fn test_multiple_empty_segments() {
        let result = canonicalize_path("/a///b").unwrap();
        assert_eq!(result.routing, "/a/b");
        assert_eq!(result.forwarding, "/a/b");
    }

    #[test]
    fn test_null_byte_rejected() {
        assert!(matches!(
            canonicalize_path("/path\u{0000}to"),
            Err(CanonicalizationError::MalformedPath)
        ));
    }

    #[test]
    fn test_control_char_rejected() {
        assert!(matches!(
            canonicalize_path("/path\u{0001}to"),
            Err(CanonicalizationError::MalformedPath)
        ));
    }

    #[test]
    fn test_del_rejected() {
        assert!(matches!(
            canonicalize_path("/path\u{007F}to"),
            Err(CanonicalizationError::MalformedPath)
        ));
    }

    #[test]
    fn test_relative_path_rejected() {
        assert!(matches!(
            canonicalize_path("relative/path"),
            Err(CanonicalizationError::MalformedPath)
        ));
    }

    #[test]
    fn test_path_with_query_still_processed() {
        let result = canonicalize_path("/path?query=1").unwrap();
        assert_eq!(result.routing, "/path?query=1");
        assert_eq!(result.original, "/path?query=1");
    }

    #[test]
    fn test_original_preserved() {
        let result = canonicalize_path("/a/../b/").unwrap();
        assert_eq!(result.routing, "/b/");
        assert_eq!(result.forwarding, "/b/");
        assert_eq!(result.original, "/a/../b/");
    }

    #[test]
    fn test_trailing_slash_with_dot_segments() {
        let result = canonicalize_path("/a/b/../c/").unwrap();
        assert_eq!(result.routing, "/a/c/");
        assert_eq!(result.forwarding, "/a/c/");
    }
}
