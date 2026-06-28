#![no_main]

use ferron_http::util::parse_q_value_header::parse_q_value_header;
use ferron_http::util::parse_q_value_header_grouped::parse_q_value_header_grouped;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let Ok(s) = core::str::from_utf8(input) else {
        return;
    };

    // Test parse_q_value_header
    let _values = parse_q_value_header(s);

    // Invariant: result must be sorted by q-value descending
    // (we can't directly check q-values since they're not returned,
    // but the function must not panic)

    // Test parse_q_value_header_grouped
    let groups = parse_q_value_header_grouped(s);

    // Invariant: each group must be non-empty
    for group in &groups {
        assert!(
            !group.is_empty(),
            "parse_q_value_header_grouped must not produce empty groups"
        );
    }

    // Invariant: groups must be ordered by q-value descending
    // (the function must not panic on any input)
});
