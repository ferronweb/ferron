#![no_main]

use ferron_http_server::util::canonicalize_url::canonicalize_path;
use libfuzzer_sys::fuzz_target;

fn assert_no_path_traversal(routing: &str, forwarding: &str) {
    assert!(
        !routing.split('/').any(|s| s == ".."),
        "routing must not contain '..' (root escape): {}",
        routing
    );
    assert!(
        !forwarding.split('/').any(|s| s == ".."),
        "forwarding must not contain '..' (root escape): {}",
        forwarding
    );
}

fn assert_no_dot_segments(s: &str, label: &str) {
    let segments: Vec<&str> = s.split('/').collect();
    for seg in &segments {
        if *seg == "." {
            panic!(
                "{} must not contain standalone '.' segments: {}",
                label, s
            );
        }
    }
}

fn assert_no_control_chars(s: &str, label: &str) {
    assert!(
        s.as_bytes().iter().all(|&b| b >= 0x20 && b != 0x7F),
        "{} must not contain null bytes or control characters: {}",
        label,
        s
    );
}

fn assert_forwarding_uppercase_hex(forwarding: &str) {
    let bytes = forwarding.as_bytes();
    for i in 0..bytes.len().saturating_sub(2) {
        if bytes[i] == b'%' && bytes[i + 1].is_ascii_hexdigit() && bytes[i + 2].is_ascii_hexdigit()
        {
            let h1 = bytes[i + 1];
            let h2 = bytes[i + 2];
            if (h1 >= b'a' && h1 <= b'f') || (h2 >= b'a' && h2 <= b'f') {
                panic!(
                    "forwarding must use uppercase hex digits: {} (found lowercase)",
                    &forwarding[i..i + 3]
                );
            }
        }
    }
}

fn assert_routing_no_unreserved_encoding(routing: &str) {
    let bytes = routing.as_bytes();
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == b'%' {
            let h1 = bytes[i + 1];
            let h2 = bytes[i + 2];
            if h1.is_ascii_hexdigit() && h2.is_ascii_hexdigit() {
                let v1 = if h1 >= b'a' {
                    h1 - b'a' + 10
                } else if h1 >= b'A' {
                    h1 - b'A' + 10
                } else {
                    h1 - b'0'
                };
                let v2 = if h2 >= b'a' {
                    h2 - b'a' + 10
                } else if h2 >= b'A' {
                    h2 - b'A' + 10
                } else {
                    h2 - b'0'
                };
                let value = (v1 << 4) | v2;
                if matches!(value, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~')
                {
                    panic!(
                        "routing contains unreserved character that should be decoded: %{} -> '{:?}'",
                        format_args!("{:01X}{:01X}", value >> 4, value & 0x0F),
                        value as char
                    );
                }
            }
        }
        i += 1;
    }
}

fn assert_no_double_encoded_percent(forwarding: &str) {
    assert!(
        !forwarding.as_bytes().windows(5).any(|w| {
            w[0] == b'%'
                && w[1] == b'2'
                && w[2] == b'5'
                && w[3].is_ascii_hexdigit()
                && w[4].is_ascii_hexdigit()
        }),
        "forwarding must not contain double-encoded percent (%25): {}",
        forwarding
    );
}

fn assert_no_excessive_nested_encoding(forwarding: &str) {
    let bytes = forwarding.as_bytes();
    let mut i = 0;
    while i + 4 < bytes.len() {
        if bytes[i] == b'%' && bytes[i + 1] == b'2' && bytes[i + 2] == b'5' {
            if bytes[i + 3].is_ascii_hexdigit() && bytes[i + 4].is_ascii_hexdigit() {
                panic!(
                    "forwarding contains excessive nested encoding: {}..{} (should reject %25xx patterns)",
                    i, i + 4
                );
            }
        }
        i += 1;
    }
}

fn assert_routing_prefix(routing: &str) {
    assert!(
        routing == "*" || routing.starts_with('/'),
        "routing must start with '/' or be '*' (got: {})",
        routing
    );
}

fn assert_forwarding_prefix(forwarding: &str) {
    assert!(
        forwarding == "*" || forwarding.starts_with('/'),
        "forwarding must start with '/' or be '*' (got: {})",
        forwarding
    );
}

fn assert_consistent_star(routing: &str, forwarding: &str) {
    if routing == "*" {
        assert_eq!(
            forwarding, "*",
            "routing is '*' but forwarding is '{}'",
            forwarding
        );
    }
}

fn assert_canonicalized_path_valid(routing: &str, forwarding: &str) {
    assert_routing_prefix(routing);
    assert_forwarding_prefix(forwarding);
    assert_consistent_star(routing, forwarding);
    assert_no_path_traversal(routing, forwarding);
    assert_no_dot_segments(routing, "routing");
    assert_no_dot_segments(forwarding, "forwarding");
    assert_no_control_chars(routing, "routing");
    assert_no_control_chars(forwarding, "forwarding");
    assert_forwarding_uppercase_hex(forwarding);
    assert_routing_no_unreserved_encoding(routing);
    assert_no_excessive_nested_encoding(forwarding);
    assert_no_double_encoded_percent(forwarding);
}

fuzz_target!(|input: &[u8]| {
    let Ok(input_str) = core::str::from_utf8(input) else {
        return;
    };

    match canonicalize_path(input_str) {
        Ok(result) => {
            assert_canonicalized_path_valid(&result.routing, &result.forwarding);
        }
        Err(_e) => {}
    }
});
