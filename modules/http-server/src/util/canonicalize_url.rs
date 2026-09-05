use std::borrow::Cow;
use std::fmt;

use smallvec::SmallVec;

/// Authoritative semantic path for routing, ACLs, cache keys, and scope checks.
#[derive(Debug, PartialEq)]
pub struct CanonicalizedPath<'a> {
    /// Authoritative semantic path for routing, ACLs, cache keys, and scope checks.
    ///
    /// For `"*"` input: exactly `"*"`
    /// For `"/"` paths:
    /// - Unreserved characters decoded
    /// - Reserved characters preserved as encoded
    /// - Dot-segments resolved
    /// - Root escape rejected
    /// - Trailing slash preserved
    pub routing: Cow<'a, str>,

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
    pub forwarding: Cow<'a, str>,

    /// Untouched client input for audit logging, HMAC verification, and debugging.
    /// Never use for routing, ACLs, or cache keys.
    pub original: Cow<'a, str>,
}

/// Errors that can occur during path canonicalization.
#[derive(Debug, Clone, PartialEq)]
pub enum CanonicalizationError {
    /// Input does not start with `/` or contains invalid characters.
    MalformedPath,
    /// Malformed percent-encoding such as `%`, `%G`, `%2`, or incomplete triplets.
    MalformedPercent,
    /// Input contains invalid percent-encoded UTF-8 sequences that cannot be processed.
    InvalidUtf8,
    /// Dot-segment resolution would escape above root (e.g., `/../admin`).
    RootEscape,
    /// Input contains a null byte (`\0`) in some form that cannot be handled.
    NullByte,
    /// Excessive nested encoding such as `%25xx` that would create a second decoding layer.
    ExcessiveEncoding,
}

impl fmt::Display for CanonicalizationError {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CanonicalizationError::MalformedPath => write!(f, "malformed request path"),
            CanonicalizationError::MalformedPercent => write!(f, "malformed percent-encoding"),
            CanonicalizationError::InvalidUtf8 => write!(f, "invalid UTF-8 in percent-encoding"),
            CanonicalizationError::RootEscape => write!(f, "path escapes above root"),
            CanonicalizationError::NullByte => write!(f, "null byte in input"),
            CanonicalizationError::ExcessiveEncoding => write!(f, "excessive nested encoding"),
        }
    }
}

impl std::error::Error for CanonicalizationError {}

/// Lookup table: hex digit value for each byte, or -1 if not a hex digit.
/// A single table load replaces branching `is_ascii_hexdigit` checks and
/// gives the decoded value at the same time.
const HEX_VAL: [i8; 256] = build_hex_val();

const fn build_hex_val() -> [i8; 256] {
    let mut t = [-1i8; 256];
    let mut c = b'0';
    while c <= b'9' {
        t[c as usize] = (c - b'0') as i8;
        c += 1;
    }
    c = b'a';
    while c <= b'f' {
        t[c as usize] = (c - b'a' + 10) as i8;
        c += 1;
    }
    c = b'A';
    while c <= b'F' {
        t[c as usize] = (c - b'A' + 10) as i8;
        c += 1;
    }
    t
}

/// Lookup table: true if the byte is an unreserved character per RFC 3986.
const UNRESERVED: [bool; 256] = build_unreserved();

const fn build_unreserved() -> [bool; 256] {
    let mut t = [false; 256];
    let mut c = b'A';
    while c <= b'Z' {
        t[c as usize] = true;
        c += 1;
    }
    c = b'a';
    while c <= b'z' {
        t[c as usize] = true;
        c += 1;
    }
    c = b'0';
    while c <= b'9' {
        t[c as usize] = true;
        c += 1;
    }
    t[b'-' as usize] = true;
    t[b'.' as usize] = true;
    t[b'_' as usize] = true;
    t[b'~' as usize] = true;
    t
}

/// Uppercases an ASCII hex digit. Only `a`-`f` need conversion because
/// `0`-`9` and `A`-`F` sort below `a`.
#[inline]
fn up(h: u8) -> u8 {
    if h >= b'a' {
        h - 32
    } else {
        h
    }
}

/// Pops the last segment from a `/`-joined buffer built by pushing
/// `'/' + segment` per kept segment. An empty buffer means `..` escapes
/// above root.
#[inline]
fn pop_segment(buf: &mut SmallVec<[u8; 128]>) -> Result<(), CanonicalizationError> {
    match memchr::memrchr(b'/', buf) {
        Some(idx) => {
            buf.truncate(idx);
            Ok(())
        }
        None => Err(CanonicalizationError::RootEscape),
    }
}

/// Canonicalizes a raw HTTP request target path.
///
/// Supports:
/// - Absolute paths beginning with `/`
/// - The special asterisk form `*` used for server-wide OPTIONS requests
pub fn canonicalize_path<'a>(
    raw_path: &'a str,
) -> Result<CanonicalizedPath<'a>, CanonicalizationError> {
    if raw_path == "*" {
        return Ok(CanonicalizedPath {
            routing: Cow::Borrowed("*"),
            forwarding: Cow::Borrowed("*"),
            original: Cow::Borrowed(raw_path),
        });
    }

    let bytes = raw_path.as_bytes();
    if bytes.is_empty() || bytes[0] != b'/' {
        return Err(CanonicalizationError::MalformedPath);
    }

    // First scan: reject control bytes and detect whether the resolving
    // paths are needed at all. Static file traffic is dominated by clean
    // ASCII paths, so this one pass decides between a memcpy fast path
    // and the resolving/decoding paths below. Control rejection comes
    // first so it beats any percent-encoding error later in the path.
    // `needs_resolve` is a conservative proxy: `//` means empty segments,
    // `/.` means a possible `.`/`..` segment (also matches dotfiles such as
    // `/.well-known`, which the resolving path still handles correctly).
    let mut has_percent = false;
    let mut needs_resolve = false;
    let mut prev = 0u8;
    for &b in bytes {
        if b < 0x20 || b == 0x7F {
            return Err(CanonicalizationError::MalformedPath);
        }
        if b == b'%' {
            has_percent = true;
        } else if prev == b'/' && (b == b'/' || b == b'.') {
            needs_resolve = true;
        }
        prev = b;
    }

    // Fast path: no encoding, no dot-segments, no duplicate slashes.
    // Both outputs equal the input, so just copy it.
    if !has_percent {
        if !needs_resolve {
            return Ok(CanonicalizedPath {
                routing: Cow::Borrowed(raw_path),
                forwarding: Cow::Borrowed(raw_path),
                original: Cow::Borrowed(raw_path),
            });
        }

        // Medium path: no percent-encoding, only dot-segment / duplicate
        // slash resolution. Routing and forwarding stay identical, so build
        // once and clone. No per-segment allocations: segments are appended
        // straight into the output buffer, `..` pops via `rfind`.
        let mut routing: SmallVec<[u8; 128]> = SmallVec::with_capacity(bytes.len());
        for seg in raw_path.split('/').skip(1) {
            if seg.is_empty() || seg == "." {
                continue;
            } else if seg == ".." {
                pop_segment(&mut routing)?;
            } else {
                routing.push(b'/');
                routing.extend_from_slice(seg.as_bytes());
            }
        }
        let trailing_slash = raw_path.ends_with('/') && raw_path != "/";
        if routing.is_empty() || trailing_slash {
            routing.push(b'/');
        }
        let routing = String::from_utf8(routing.into_vec())
            .map_err(|_| CanonicalizationError::InvalidUtf8)?;
        let forwarding = routing.clone();
        return Ok(CanonicalizedPath {
            routing: Cow::Owned(routing),
            forwarding: Cow::Owned(forwarding),
            original: Cow::Borrowed(raw_path),
        });
    }

    // Slow path: percent-encoding present. Triplet validation runs over the
    // whole path first (rather than per segment) without changing behavior:
    // `/` is never a hex digit, so triplets and `%25xx` patterns cannot
    // span segments. Validating up front preserves the original error
    // precedence (malformed/double encoding beats `RootEscape`).
    // Decoding and dot-segment resolution then happen in a single pass over
    // each segment, decoding directly into the two output buffers
    // (no per-segment `String` allocations, no separate decode/resolve
    // passes, no double dot resolution). The checks in the decode loop
    // re-verify defensively.
    // The routing and forwarding views resolve independently: an encoded
    // dot such as `%2E` is a dot-segment for routing but opaque data for
    // forwarding.
    {
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] != b'%' {
                i += 1;
                continue;
            }
            // Need at least 2 more bytes for a valid triplet.
            if i + 2 >= bytes.len() {
                return Err(CanonicalizationError::MalformedPercent);
            }
            let h1 = bytes[i + 1];
            let h2 = bytes[i + 2];
            if HEX_VAL[h1 as usize] < 0 || HEX_VAL[h2 as usize] < 0 {
                return Err(CanonicalizationError::MalformedPercent);
            }
            // Excessive encoding: `%25` followed by two hex digits would
            // decode to `%xx` on a second pass, which is dangerous.
            if h1 == b'2'
                && h2 == b'5'
                && i + 4 < bytes.len()
                && HEX_VAL[bytes[i + 3] as usize] >= 0
                && HEX_VAL[bytes[i + 4] as usize] >= 0
            {
                return Err(CanonicalizationError::ExcessiveEncoding);
            }
            // Null byte encoding (`%00`).
            if h1 == b'0' && h2 == b'0' {
                return Err(CanonicalizationError::NullByte);
            }
            i += 3;
        }
    }
    let mut routing: SmallVec<[u8; 128]> = SmallVec::with_capacity(bytes.len());
    let mut forwarding: SmallVec<[u8; 128]> = SmallVec::with_capacity(bytes.len());
    for seg in raw_path.split('/').skip(1) {
        if seg.is_empty() {
            continue;
        }
        let sb = seg.as_bytes();
        if !sb.contains(&b'%') {
            if seg == "." {
                continue;
            } else if seg == ".." {
                pop_segment(&mut routing)?;
                pop_segment(&mut forwarding)?;
            } else {
                routing.push(b'/');
                routing.extend_from_slice(sb);
                forwarding.push(b'/');
                forwarding.extend_from_slice(sb);
            }
            continue;
        }

        // Encoded segment: tentatively append `'/' + decoded` to both
        // buffers, then apply each view's dot rule by truncating back to
        // the mark when the decoded segment is `.`/`..`.
        let r_mark = routing.len();
        routing.push(b'/');
        forwarding.push(b'/');
        // Start of the current raw run (bytes with no `%`), flushed with
        // `push_str` so multi-byte UTF-8 is never split into chars.
        let mut run = 0;
        let mut i = 0;
        while i < sb.len() {
            if sb[i] != b'%' {
                i += 1;
                continue;
            }
            if run < i {
                // `run..i` holds no `%` (ASCII), so the bounds are UTF-8 safe.
                routing.extend_from_slice(&sb[run..i]);
                forwarding.extend_from_slice(&sb[run..i]);
            }
            // Need at least 2 more bytes for a valid triplet.
            if i + 2 >= sb.len() {
                return Err(CanonicalizationError::MalformedPercent);
            }
            let h1 = sb[i + 1];
            let h2 = sb[i + 2];
            let v1 = HEX_VAL[h1 as usize];
            let v2 = HEX_VAL[h2 as usize];
            if v1 < 0 || v2 < 0 {
                return Err(CanonicalizationError::MalformedPercent);
            }
            // Excessive encoding: `%25` followed by two hex digits would
            // decode to `%xx` on a second pass, which is dangerous.
            if h1 == b'2'
                && h2 == b'5'
                && i + 4 < sb.len()
                && HEX_VAL[sb[i + 3] as usize] >= 0
                && HEX_VAL[sb[i + 4] as usize] >= 0
            {
                return Err(CanonicalizationError::ExcessiveEncoding);
            }
            // Null byte encoding (`%00`).
            if h1 == b'0' && h2 == b'0' {
                return Err(CanonicalizationError::NullByte);
            }
            let value = ((v1 as u8) << 4) | (v2 as u8);
            let u1 = up(h1);
            let u2 = up(h2);
            if UNRESERVED[value as usize] {
                // Decode unreserved characters for routing; forwarding
                // keeps the encoding but uppercased.
                routing.push(value);
                forwarding.push(b'%');
                forwarding.push(u1);
                forwarding.push(u2);
            } else {
                // Reserved characters stay encoded in both views.
                routing.push(b'%');
                routing.push(u1);
                routing.push(u2);
                forwarding.push(b'%');
                forwarding.push(u1);
                forwarding.push(u2);
            }
            i += 3;
            run = i;
        }
        if run < sb.len() {
            routing.extend_from_slice(&sb[run..]);
            forwarding.extend_from_slice(&sb[run..]);
        }

        // Per-view dot rules on the just-decoded segment. The forwarding
        // view of an encoded segment always contains `%`, so it can never
        // be `.`/`..`/empty and is kept as appended.
        // `r_mark + 1` skips the `'/'` just pushed (ASCII boundary).
        let r_seg = &routing[r_mark + 1..];
        if r_seg == b"." {
            routing.truncate(r_mark);
        } else if r_seg == b".." {
            routing.truncate(r_mark);
            pop_segment(&mut routing)?;
        }
    }

    // Track trailing slash: present if path ends with `/` and is not just `"/"`.
    let trailing_slash = raw_path.ends_with('/') && raw_path != "/";
    if routing.is_empty() || trailing_slash {
        routing.push(b'/');
    }
    if forwarding.is_empty() || trailing_slash {
        forwarding.push(b'/');
    }

    Ok(CanonicalizedPath {
        routing: Cow::Owned(
            String::from_utf8(routing.into_vec())
                .map_err(|_| CanonicalizationError::InvalidUtf8)?,
        ),
        forwarding: Cow::Owned(
            String::from_utf8(forwarding.into_vec())
                .map_err(|_| CanonicalizationError::InvalidUtf8)?,
        ),
        original: Cow::Borrowed(raw_path),
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

    #[test]
    fn test_utf8() {
        let result = canonicalize_path("/wziąść").unwrap();
        assert_eq!(result.routing, "/wziąść");
        assert_eq!(result.forwarding, "/wziąść");
        assert_eq!(result.original, "/wziąść");
    }

    #[test]
    fn test_utf8_encoded() {
        // wzi%C4%85%C5%9B%C4%87 -> wziąść
        let result = canonicalize_path("/wzi%C4%85%C5%9B%C4%87").unwrap();
        assert_eq!(result.routing, "/wzi%C4%85%C5%9B%C4%87");
        assert_eq!(result.forwarding, "/wzi%C4%85%C5%9B%C4%87");
        assert_eq!(result.original, "/wzi%C4%85%C5%9B%C4%87");
    }
}
