#![no_main]

use ferron_http_server::util::canonicalize_url::canonicalize_path_routing;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let Ok(input_str) = core::str::from_utf8(input) else {
        return;
    };

    let result = canonicalize_path_routing(input_str);

    if let Ok((routing, original)) = result {
        assert_canonicalized_routing_valid(&routing, &original, input_str);
    }
});

#[inline]
fn assert_canonicalized_routing_valid(routing: &str, original: &str, input: &str) {
    assert!(
        routing == "*" || routing.starts_with('/'),
        "routing must start with '/' or be '*', got: {}",
        routing
    );

    assert!(
        !routing.split('/').any(|s| s == ".."),
        "routing must not contain '..' (root escape), got: {}",
        routing
    );

    let segments: Vec<&str> = routing.split('/').collect();
    for seg in &segments[1..(segments.len() - 1).max(1)] {
        assert!(
            !seg.is_empty() && *seg != ".",
            "routing segment '{}' should not be '.' or empty",
            seg
        );
    }

    assert!(
        !routing.contains('\0'),
        "routing must not contain null bytes, got: {:?}",
        routing
    );

    for (i, c) in routing.chars().enumerate() {
        let code = c as u32;
        assert!(
            !(code < 0x20 || code == 0x7F),
            "routing must not contain control characters at position {}, got: {}",
            i,
            routing
        );
    }

    let three_chars = routing
        .chars()
        .zip(routing.chars().skip(1))
        .zip(routing.chars().skip(2))
        .map(|((c1, c2), c3)| (c1, c2, c3));
    for (i, (c1, c2, c3)) in three_chars.enumerate() {
        if c1 == '%' {
            let c2s = c2.to_string();
            let c3s = c3.to_string();
            if let (Some(h1), Some(h2)) = (
                u8::from_str_radix(&c2s, 16).ok(),
                u8::from_str_radix(&c3s, 16).ok(),
            ) {
                let value = (h1 << 4) | h2;
                if is_unreserved(value) {
                    assert!(
                        false,
                        "routing must decode unreserved characters, found '%{:01X}{:01X}' at position {} (should be '{}')",
                        h1, h2, i, value as char
                    );
                }
            }
        }
    }

    assert!(
        !routing.contains("%00"),
        "routing must not contain %00 (null byte encoding), got: {}",
        routing
    );

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
                    c2, i + 1
                );
            } else if c3.is_ascii_lowercase() {
                assert!(
                    false,
                    "routing hex digits must be uppercase, found lowercase '{}' at position {}",
                    c3, i + 2
                );
            }
        }
    }

    assert_eq!(
        original, input,
        "original must equal input, got original: '{}', input: '{}'",
        original, input
    );

    if routing == "*" {
        assert_eq!(
            original, "*",
            "If routing is '*', original must also be '*'"
        );
    }
}

#[inline]
fn is_unreserved(value: u8) -> bool {
    matches!(
        value,
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~'
    )
}
