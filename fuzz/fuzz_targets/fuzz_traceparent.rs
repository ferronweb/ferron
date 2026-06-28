#![no_main]

use ferron_http::trace_context::parse_traceparent;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let Ok(s) = core::str::from_utf8(input) else {
        return;
    };

    if let Some(tc) = parse_traceparent(s) {
        // Invariant 1: trace_id must be exactly 32 hex chars
        assert_eq!(
            tc.trace_id.len(),
            32,
            "trace_id must be 32 hex chars, got {}",
            tc.trace_id.len()
        );
        // Invariant 2: span_id must be exactly 16 hex chars
        assert_eq!(
            tc.span_id.len(),
            16,
            "span_id must be 16 hex chars, got {}",
            tc.span_id.len()
        );
        // Invariant 3: both must be lowercase hex
        assert!(
            tc.trace_id
                .chars()
                .all(|c| c.is_ascii_hexdigit() && c.is_ascii_lowercase()),
            "trace_id must be lowercase hex: {}",
            tc.trace_id
        );
        assert!(
            tc.span_id
                .chars()
                .all(|c| c.is_ascii_hexdigit() && c.is_ascii_lowercase()),
            "span_id must be lowercase hex: {}",
            tc.span_id
        );
    }
});
