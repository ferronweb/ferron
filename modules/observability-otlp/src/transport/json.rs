use serde_json::Value;

/// Serialize an OTLP request message into its JSON Protobuf representation
/// (OTLP/HTTP JSON encoding).
///
/// `pbjson` follows the standard Protobuf JSON mapping. OTLP/HTTP JSON
/// deviates from it in one way relevant to the messages Ferron emits: the
/// `traceId`, `spanId`, and `parentSpanId` byte fields must be hex-encoded
/// strings instead of base64 (see the OTLP specification, "JSON Protobuf
/// Encoding"). [`hexify_id_fields`] rewrites those fields after serialization.
#[inline]
pub fn request_to_json<T: serde::Serialize>(message: &T) -> Value {
    let mut value =
        serde_json::to_value(message).expect("OTLP request JSON serialization must not fail");
    hexify_id_fields(&mut value);
    value
}

/// Recursively rewrite `traceId`, `spanId`, and `parentSpanId` string fields
/// from base64 (standard Protobuf JSON mapping) to uppercase hex (OTLP JSON
/// mapping).
///
/// These are the only `bytes` fields in the OTLP telemetry messages Ferron
/// emits, so any string value under one of these keys was base64-encoded by
/// `pbjson`. Strings that cannot be base64-decoded are left untouched.
#[inline]
pub fn hexify_id_fields(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, field) in map.iter_mut() {
                if matches!(key.as_str(), "traceId" | "spanId" | "parentSpanId") {
                    if let Value::String(encoded) = field {
                        if let Ok(bytes) = base64::Engine::decode(
                            &base64::engine::general_purpose::STANDARD,
                            encoded,
                        ) {
                            *field = Value::String(hex::encode_upper(bytes));
                        }
                    }
                } else {
                    hexify_id_fields(field);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                hexify_id_fields(item);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::opentelemetry::proto::{
        collector::trace::v1::ExportTraceServiceRequest,
        common::v1::{any_value, AnyValue, InstrumentationScope, KeyValue},
        logs::v1::LogRecord,
        metrics::v1::{exemplar, Exemplar},
        resource::v1::Resource,
        trace::v1::{span, status, ResourceSpans, ScopeSpans, Span, Status},
    };

    /// Decode an uppercase hex string into raw bytes.
    fn hex_bytes(hex: &str) -> Vec<u8> {
        hex::decode(hex).unwrap()
    }

    #[test]
    fn span_json_uses_hex_encoded_ids_and_integer_enums() {
        let span = Span {
            trace_id: hex_bytes("5B8EFFF798038103D269B633813FC60C"),
            span_id: hex_bytes("EEE19B7EC3C1B174"),
            parent_span_id: hex_bytes("EEE19B7EC3C1B173"),
            name: "I'm a server span".to_string(),
            kind: span::SpanKind::Server as i32,
            start_time_unix_nano: 1544712660000000000,
            end_time_unix_nano: 1544712661000000000,
            ..Default::default()
        };

        let json = request_to_json(&span);

        // OTLP JSON encoding: trace/span/parent IDs are hex strings, not base64.
        assert_eq!(json["traceId"], "5B8EFFF798038103D269B633813FC60C");
        assert_eq!(json["spanId"], "EEE19B7EC3C1B174");
        assert_eq!(json["parentSpanId"], "EEE19B7EC3C1B173");

        // Enums are encoded as integer values.
        assert_eq!(json["kind"], 2);

        // 64-bit integers are encoded as decimal strings.
        assert_eq!(json["startTimeUnixNano"], "1544712660000000000");
    }

    #[test]
    fn exemplar_json_uses_hex_encoded_ids() {
        let exemplar = Exemplar {
            time_unix_nano: 1544712660300000000,
            span_id: hex_bytes("EEE19B7EC3C1B174"),
            trace_id: hex_bytes("5B8EFFF798038103D269B633813FC60C"),
            value: Some(exemplar::Value::AsDouble(5.0)),
            ..Default::default()
        };

        let json = request_to_json(&exemplar);

        assert_eq!(json["traceId"], "5B8EFFF798038103D269B633813FC60C");
        assert_eq!(json["spanId"], "EEE19B7EC3C1B174");
        assert_eq!(json["asDouble"], 5.0);
    }

    #[test]
    fn log_record_json_uses_hex_encoded_ids() {
        let log_record = LogRecord {
            time_unix_nano: 1544712660300000000,
            observed_time_unix_nano: 1544712660300000000,
            severity_number: 9,
            severity_text: "INFO".to_string(),
            trace_id: hex_bytes("5B8EFFF798038103D269B633813FC60C"),
            span_id: hex_bytes("EEE19B7EC3C1B174"),
            body: Some(AnyValue {
                value: Some(any_value::Value::StringValue("some log".to_string())),
            }),
            ..Default::default()
        };

        let json = request_to_json(&log_record);

        assert_eq!(json["traceId"], "5B8EFFF798038103D269B633813FC60C");
        assert_eq!(json["spanId"], "EEE19B7EC3C1B174");
        assert_eq!(json["severityNumber"], 9);
        assert_eq!(json["severityText"], "INFO");
        assert_eq!(json["body"]["stringValue"], "some log");
    }

    #[test]
    fn span_json_roundtrips_with_standard_protobuf_json_mapping() {
        // The OTLP JSON encoding is not symmetric under `pbjson`
        // deserialization (IDs are hex, while `pbjson` expects base64), so the
        // round-trip is verified against the standard mapping produced by
        // `pbjson` before the ID rewrite.
        let span = Span {
            trace_id: hex_bytes("5B8EFFF798038103D269B633813FC60C"),
            span_id: hex_bytes("EEE19B7EC3C1B174"),
            name: "round trip".to_string(),
            kind: span::SpanKind::Internal as i32,
            status: Some(Status {
                code: status::StatusCode::Error as i32,
                message: "boom".to_string(),
            }),
            attributes: vec![KeyValue {
                key: "http.request.method".to_string(),
                value: Some(AnyValue {
                    value: Some(any_value::Value::StringValue("GET".to_string())),
                }),
                ..Default::default()
            }],
            ..Default::default()
        };

        let json = serde_json::to_value(&span).unwrap();
        let decoded: Span = serde_json::from_value(json).unwrap();

        assert_eq!(decoded, span);
    }

    #[test]
    fn span_json_matches_official_example() {
        // Golden fixture from the `opentelemetry-proto` repository
        // (examples/trace.json), exercising the OTLP/HTTP JSON encoding
        // deviations: hex IDs, integer enums, and string-encoded 64-bit
        // integers.
        let fixture: Value = serde_json::from_str(include_str!(
            "../../opentelemetry-proto/examples/trace.json"
        ))
        .unwrap();

        let request = ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                resource: Some(Resource {
                    attributes: vec![KeyValue {
                        key: "service.name".to_string(),
                        value: Some(AnyValue {
                            value: Some(any_value::Value::StringValue("my.service".to_string())),
                        }),
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
                scope_spans: vec![ScopeSpans {
                    scope: Some(InstrumentationScope {
                        name: "my.library".to_string(),
                        version: "1.0.0".to_string(),
                        attributes: vec![KeyValue {
                            key: "my.scope.attribute".to_string(),
                            value: Some(AnyValue {
                                value: Some(any_value::Value::StringValue(
                                    "some scope attribute".to_string(),
                                )),
                            }),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                    spans: vec![Span {
                        trace_id: hex_bytes("5B8EFFF798038103D269B633813FC60C"),
                        span_id: hex_bytes("EEE19B7EC3C1B174"),
                        parent_span_id: hex_bytes("EEE19B7EC3C1B173"),
                        name: "I'm a server span".to_string(),
                        kind: span::SpanKind::Server as i32,
                        start_time_unix_nano: 1544712660000000000,
                        end_time_unix_nano: 1544712661000000000,
                        attributes: vec![KeyValue {
                            key: "my.span.attr".to_string(),
                            value: Some(AnyValue {
                                value: Some(any_value::Value::StringValue(
                                    "some value".to_string(),
                                )),
                            }),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };

        let json = request_to_json(&request);

        assert_eq!(json, fixture);
    }
}
