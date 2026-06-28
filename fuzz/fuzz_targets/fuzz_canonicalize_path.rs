#![no_main]

use ferron_http_server::util::canonicalize_url::canonicalize_path;
use libfuzzer_sys::fuzz_target;

fn assert_canonicalized_path_valid(routing: &str, forwarding: &str) {
    assert!(
        routing == "*" || routing.starts_with('/'),
        "routing must start with '/' or be '*' (got: {})",
        routing
    );
    assert!(
        forwarding == "*" || forwarding.starts_with('/'),
        "forwarding must start with '/' or be '*' (got: {})",
        forwarding
    );

    if routing == "*" {
        assert_eq!(
            forwarding, "*",
            "routing is '*' but forwarding is '{}'",
            forwarding
        );
    }

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

    let check_dot_segments = |s: &str| {
        let segments: Vec<&str> = s.split('/').collect();
        for seg in segments {
            if seg == "." {
                return false;
            }
        }
        true
    };
    assert!(
        check_dot_segments(routing),
        "routing must not contain standalone '.' segments: {}",
        routing
    );
    assert!(
        check_dot_segments(forwarding),
        "forwarding must not contain standalone '.' segments: {}",
        forwarding
    );

    assert!(
        routing.as_bytes().iter().all(|&b| b >= 0x20 && b != 0x7F),
        "routing must not contain null bytes or control characters: {}",
        routing
    );
    assert!(
        forwarding
            .as_bytes()
            .iter()
            .all(|&b| b >= 0x20 && b != 0x7F),
        "forwarding must not contain null bytes or control characters: {}",
        forwarding
    );

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

    let check_routing_encoding = |s: &str| {
        let bytes = s.as_bytes();
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
                        return Some(value);
                    }
                }
            }
            i += 1;
        }
        None
    };

    if let Some(decoded) = check_routing_encoding(routing) {
        panic!(
            "routing contains unreserved character that should be decoded: %{} -> '{:?}'",
            format_args!("{:01X}{:01X}", decoded >> 4, decoded & 0x0F),
            decoded as char
        );
    }

    let check_excessive_encoding = |s: &str| {
        let bytes = s.as_bytes();
        let mut i = 0;
        while i + 4 < bytes.len() {
            if bytes[i] == b'%' && bytes[i + 1] == b'2' && bytes[i + 2] == b'5' {
                if bytes[i + 3].is_ascii_hexdigit() && bytes[i + 4].is_ascii_hexdigit() {
                    return Some((i, i + 4));
                }
            }
            i += 1;
        }
        None
    };

    if let Some((start, end)) = check_excessive_encoding(forwarding) {
        panic!(
            "forwarding contains excessive nested encoding: {}..{} (should reject %25xx patterns)",
            start, end
        );
    }

    let check_double_encoded_percent = |s: &str| {
        s.as_bytes().windows(5).any(|w| {
            w[0] == b'%'
                && w[1] == b'2'
                && w[2] == b'5'
                && w[3].is_ascii_hexdigit()
                && w[4].is_ascii_hexdigit()
        })
    };

    assert!(
        !check_double_encoded_percent(forwarding),
        "forwarding must not contain double-encoded percent (%25): {}",
        forwarding
    );
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
