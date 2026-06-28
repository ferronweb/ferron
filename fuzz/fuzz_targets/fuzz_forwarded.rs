#![no_main]

use std::net::IpAddr;

use libfuzzer_sys::fuzz_target;

/// Split a `Forwarded` header value into individual forwarded elements,
/// respecting quoted strings.
fn split_forwarded_elements(value: &str) -> Vec<&str> {
    let mut elements = Vec::new();
    let mut current_start = 0;
    let mut in_quotes = false;

    for (i, ch) in value.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                elements.push(value[current_start..i].trim());
                current_start = i + 1;
            }
            _ => {}
        }
    }

    let remainder = value[current_start..].trim();
    if !remainder.is_empty() {
        elements.push(remainder);
    }

    elements
}

/// Find a parameter value in a forwarded element.
fn find_forwarded_param<'a>(element: &'a str, param_name: &str) -> Option<&'a str> {
    let prefix = format!("{param_name}=");
    for part in element.split(';') {
        let part = part.trim();
        if let Some(val) = part.strip_prefix(&prefix) {
            return Some(val.trim());
        }
    }
    None
}

/// Extract client IP from Forwarded header (RFC 7239).
fn extract_forwarded_for(value: &str) -> Option<IpAddr> {
    let first_element = split_forwarded_elements(value).first().copied()?;
    let for_value = find_forwarded_param(first_element, "for")?;

    let unquoted = for_value
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(for_value);

    let cleaned = unquoted
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(unquoted);

    cleaned.parse::<IpAddr>().ok()
}

fuzz_target!(|input: &[u8]| {
    let Ok(s) = core::str::from_utf8(input) else {
        return;
    };

    // Exercise split_forwarded_elements
    let elements = split_forwarded_elements(s);

    // Invariant: all elements must be non-empty after trimming
    for elem in &elements {
        assert!(
            !elem.is_empty(),
            "split_forwarded_elements produced empty element"
        );
    }

    // Invariant: elements don't overlap and cover the full input
    // (this is a structural property, not easily checked with arbitrary input)

    // Exercise extract_forwarded_for
    let _result = extract_forwarded_for(s);

    // Exercise find_forwarded_param
    let _param = find_forwarded_param(s, "for");
    let _proto = find_forwarded_param(s, "proto");
    let _by = find_forwarded_param(s, "by");
    let _host = find_forwarded_param(s, "host");
});
