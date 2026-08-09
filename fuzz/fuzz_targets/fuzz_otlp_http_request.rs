//! Fuzz target for the OTLP/HTTP request encoding paths
//! (CUSTOM_EXPORTER_REWRITE.md §7.3).
//!
//! Exercises:
//! - the protobuf decode → encode → decode round-trip of an
//!   `ExportTraceServiceRequest`;
//! - the OTLP/HTTP JSON encoding (`request_to_json`, including the hex-ID
//!   rewrite) — must not panic, must be deterministic, and must re-serialize;
//! - `hexify_id_fields` over arbitrary JSON values — must not panic and must
//!   be idempotent.

#![no_main]

use ferron_observability_otlp::proto::opentelemetry::proto::collector::trace::v1::ExportTraceServiceRequest;
use ferron_observability_otlp::transport::json::{hexify_id_fields, request_to_json};
use libfuzzer_sys::fuzz_target;
use prost::Message;

fuzz_target!(|data: &[u8]| {
    if let Ok(request) = ExportTraceServiceRequest::decode(data) {
        // Protobuf encode round-trip: encode must succeed and decode must
        // reproduce the original request exactly.
        let wire = request.encode_to_vec();
        match ExportTraceServiceRequest::decode(wire.as_slice()) {
            Ok(roundtrip) => assert_eq!(roundtrip, request, "encode/decode round-trip drift"),
            Err(error) => panic!("encoded request failed to decode: {error}"),
        }

        // OTLP/HTTP JSON encoding: must not panic, must be deterministic,
        // and must serialize back to JSON.
        let json = request_to_json(&request);
        assert!(
            serde_json::to_vec(&json).is_ok(),
            "OTLP JSON payload failed to re-serialize"
        );
        assert_eq!(request_to_json(&request), json, "JSON encoding not deterministic");
    }

    // The hex-ID rewrite applied to arbitrary JSON values must be idempotent.
    let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(data) else {
        return;
    };
    hexify_id_fields(&mut value);
    let rewritten = value.clone();
    hexify_id_fields(&mut value);
    assert_eq!(value, rewritten, "hexify_id_fields is not idempotent");
});