//! Unified TLS observability helpers shared by every TLS provider.
//!
//! All certificate providers (`manual`, `acme`, `http`, `local`) emit a
//! single, consistent gauge once a certificate is mounted into the in-memory
//! `rustls` context. The metric name, attribute set, and value semantics are
//! owned by this module so providers do not need to know them.

use std::sync::Arc;
use std::time::Duration;

use ferron_observability::{
    CompositeEventSink, Event, MetricAttributeValue, MetricEvent, MetricType, MetricValue,
};
use rustls_pki_types::CertificateDer;
use x509_parser::certificate::X509Certificate;
use x509_parser::prelude::FromDer;

/// Name of the unified certificate `notAfter` gauge emitted by every TLS
/// provider when a certificate is mounted into the in-memory context.
pub const METRIC_NAME: &str = "ferron.tls.certificate_not_after";

/// Emit a `ferron.tls.certificate_not_after` gauge for a freshly mounted
/// certificate.
///
/// - `provider` — a `&'static str` matching the TLS provider name
///   (`"manual"`, `"acme"`, `"http"`, or `"local"`).
/// - `host` — the SNI hostname or IP literal the certificate is bound to.
/// - `leaf` — the first (end-entity) certificate of the chain.
///
/// If the leaf cannot be parsed as an X.509 certificate the function is a
/// silent no-op; the rest of the code path will fail to install the cert
/// anyway, so there is nothing useful to report here.
pub fn emit_certificate_not_after(
    event_sink: &Arc<CompositeEventSink>,
    provider: &'static str,
    host: &str,
    leaf: &CertificateDer<'_>,
) {
    if event_sink.is_empty() {
        return;
    }

    let Ok((_, cert)) = X509Certificate::from_der(leaf.as_ref()) else {
        return;
    };

    let not_after = match cert.validity().not_after.timestamp() {
        ts if ts < 0 => 0u64,
        ts => ts as u64,
    };
    let serial_hex = cert.tbs_certificate.serial.to_str_radix(16);

    event_sink.emit(Event::Metric(MetricEvent {
        name: METRIC_NAME,
        attributes: vec![
            (
                "ferron.host",
                MetricAttributeValue::String(host.to_string()),
            ),
            (
                "ferron.tls.provider",
                MetricAttributeValue::StaticStr(provider),
            ),
            (
                "crypto.certificate.serial_number",
                MetricAttributeValue::String(serial_hex),
            ),
        ],
        ty: MetricType::Gauge,
        value: MetricValue::U64(not_after),
        unit: Some("s"),
        description: Some("Certificate `notAfter` field as Unix epoch seconds"),
        trace_context: None,
        control_plane_metadata: None,
    }));
}

/// Name of the TLS handshake duration histogram.
pub const HANDSHAKE_DURATION_METRIC: &str = "ferron.tls.handshake.duration";

/// Name of the TLS handshake total counter.
pub const HANDSHAKE_TOTAL_METRIC: &str = "ferron.tls.handshake.total";

/// Name of the active TLS connections gauge.
pub const CONNECTIONS_ACTIVE_METRIC: &str = "ferron.tls.connections.active";

/// Emit a `ferron.tls.handshake.duration` histogram for a TLS handshake.
///
/// - `event_sink` — the per-host `CompositeEventSink`.
/// - `host` — the SNI hostname or `"_global"` for connections without SNI.
/// - `duration` — elapsed time for the handshake.
/// - `protocol_version` — negotiated protocol (e.g. `"TLSv1.3"`).
/// - `cipher_suite` — negotiated cipher suite (e.g. `"TLS_AES_256_GCM_SHA384"`).
/// - `result` — `"success"` or `"error"`.
pub fn emit_handshake_duration(
    event_sink: &CompositeEventSink,
    host: &str,
    duration: Duration,
    protocol_version: &str,
    cipher_suite: &str,
    result: &'static str,
) {
    if event_sink.is_empty() {
        return;
    }

    event_sink.emit(Event::Metric(MetricEvent {
        name: HANDSHAKE_DURATION_METRIC,
        attributes: vec![
            (
                "ferron.host",
                MetricAttributeValue::String(host.to_string()),
            ),
            (
                "tls.protocol.version",
                MetricAttributeValue::String(protocol_version.to_string()),
            ),
            (
                "tls.cipher_suite",
                MetricAttributeValue::String(cipher_suite.to_string()),
            ),
            (
                "ferron.tls.handshake.result",
                MetricAttributeValue::StaticStr(result),
            ),
        ],
        ty: MetricType::Histogram(None),
        value: MetricValue::F64(duration.as_secs_f64()),
        unit: Some("s"),
        description: Some("TLS handshake latency"),
        trace_context: None,
        control_plane_metadata: None,
    }));
}

/// Emit a `ferron.tls.handshake.total` counter for a TLS handshake attempt.
pub fn emit_handshake_total(event_sink: &CompositeEventSink, host: &str, result: &'static str) {
    if event_sink.is_empty() {
        return;
    }

    event_sink.emit(Event::Metric(MetricEvent {
        name: HANDSHAKE_TOTAL_METRIC,
        attributes: vec![
            (
                "ferron.host",
                MetricAttributeValue::String(host.to_string()),
            ),
            (
                "ferron.tls.handshake.result",
                MetricAttributeValue::StaticStr(result),
            ),
        ],
        ty: MetricType::Counter,
        value: MetricValue::U64(1),
        unit: Some("{handshake}"),
        description: Some("Total TLS handshake attempts"),
        trace_context: None,
        control_plane_metadata: None,
    }));
}

/// Emit a `ferron.tls.connections.active` up-down counter delta.
pub fn emit_connections_active(event_sink: &CompositeEventSink, host: &str, delta: i64) {
    if event_sink.is_empty() {
        return;
    }

    event_sink.emit(Event::Metric(MetricEvent {
        name: CONNECTIONS_ACTIVE_METRIC,
        attributes: vec![(
            "ferron.host",
            MetricAttributeValue::String(host.to_string()),
        )],
        ty: MetricType::UpDownCounter,
        value: MetricValue::I64(delta),
        unit: Some("{connection}"),
        description: Some("Currently open TLS connections"),
        trace_context: None,
        control_plane_metadata: None,
    }));
}
