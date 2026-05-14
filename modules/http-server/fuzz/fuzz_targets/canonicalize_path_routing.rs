#![no_main]

use ferron_http_server::util::canonicalize_url::canonicalize_path_routing;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    // Convert input bytes to UTF-8 string
    let Ok(input_str) = core::str::from_utf8(input) else {
        return; // Non-UTF8 input - canonicalizer expects UTF-8 strings
    };

    // Exercise the canonicalizer
    let result = canonicalize_path_routing(input_str);

    // Assert semantic properties of successful canonicalization
    if let Ok((routing, original)) = result {
        assert_canonicalized_routing_valid(&routing, &original, input_str);
    }
});

/// Asserts all security invariants for canonicalized routing paths.
///
/// These checks detect bypasses where dangerous URL sequences escape validation:
/// - Root escape via `..`
/// - Null byte injection
/// - Control character leakage
/// - Incomplete unreserved character decoding
/// - Overlong percent-encoding
/// - Encoding inconsistencies
#[inline]
fn assert_canonicalized_routing_valid(routing: &str, original: &str, input: &str) {
    // Invariant 1: Must start with `/` or be `*`
    assert!(
        routing == "*" || routing.starts_with('/'),
        "routing must start with '/' or be '*', got: {}",
        routing
    );

    // Invariant 2: Must NOT contain `..` (root escape bypass)
    assert!(
        !routing.split('/').any(|s| s == ".."),
        "routing must not contain '..' (root escape), got: {}",
        routing
    );

    // Invariant 3: Must NOT contain `.` as a standalone segment
    // Check for segments that are exactly "."
    let segments: Vec<&str> = routing.split('/').collect();
    for seg in &segments[1..(segments.len() - 1).max(1)] {
        assert!(
            !seg.is_empty() && *seg != ".",
            "routing segment '{}' should not be '.' or empty",
            seg
        );
    }

    // Invariant 4: Must NOT contain null bytes
    assert!(
        !routing.contains('\0'),
        "routing must not contain null bytes, got: {:?}",
        routing
    );

    // Invariant 5: Must NOT contain control characters (< 0x20 or 0x7F)
    for (i, c) in routing.chars().enumerate() {
        let code = c as u32;
        assert!(
            !(code < 0x20 || code == 0x7F),
            "routing must not contain control characters at position {}, got: {}",
            i,
            routing
        );
    }

    // Invariant 6: Must NOT contain un-decoded unreserved characters
    // (e.g., %41 should be 'A' in routing, not '%41')

    let three_chars = routing
        .chars()
        .zip(routing.chars().skip(1))
        .zip(routing.chars().skip(2))
        .map(|((c1, c2), c3)| (c1, c2, c3));
    for (i, (c1, c2, c3)) in three_chars.enumerate() {
        if c1 == '%' {
            // Check if this is a percent-encoded unreserved character
            let c2s = c2.to_string();
            let c3s = c3.to_string();
            if let (Some(h1), Some(h2)) = (
                u8::from_str_radix(&c2s, 16).ok(),
                u8::from_str_radix(&c3s, 16).ok(),
            ) {
                let value = (h1 << 4) | h2;
                // Check if it's an unreserved character that should be decoded
                if is_unreserved(value) {
                    // This is a bug: unreserved chars should be decoded in routing
                    assert!(
                                    false,
                                    "routing must decode unreserved characters, found '%{:01X}{:01X}' at position {} (should be '{}')",
                                    h1,
                                    h2,
                                    i,
                                    value as char
                                );
                }
            }
        }
    }

    // Invariant 7: Must NOT have percent-encoded null bytes (%00)
    assert!(
        !routing.contains("%00"),
        "routing must not contain %00 (null byte encoding), got: {}",
        routing
    );

    // Invariant 8: Hex digits in %xx must be uppercase (determinism)
    let three_chars = routing
        .chars()
        .zip(routing.chars().skip(1))
        .zip(routing.chars().skip(2))
        .map(|((c1, c2), c3)| (c1, c2, c3));
    for (i, (c1, c2, c3)) in three_chars.enumerate() {
        if c1 == '%' {
            if c2.is_ascii_lowercase() {
                assert!(
                    false,
                    "routing hex digits must be uppercase, found lowercase '{}' at position {}",
                    c2,
                    i + 1
                );
            } else if c3.is_ascii_lowercase() {
                assert!(
                    false,
                    "routing hex digits must be uppercase, found lowercase '{}' at position {}",
                    c3,
                    i + 2
                );
            }
        }
    }

    // Invariant 9: Original must match input (preservation)
    assert_eq!(
        original, input,
        "original must equal input, got original: '{}', input: '{}'",
        original, input
    );

    // Invariant 10: Asterisk form consistency
    if routing == "*" {
        assert_eq!(
            original, "*",
            "If routing is '*', original must also be '*'"
        );
    }
}

/// Check if a byte value is an unreserved character per RFC 3986.
///
/// Unreserved characters: A-Z a-z 0-9 - . _ ~
#[inline]
fn is_unreserved(value: u8) -> bool {
    matches!(
        value,
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~'
    )
}
