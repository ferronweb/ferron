use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::SystemTime;

use ferron_core::config::ServerConfigurationBlock;
use ferron_core::registry::Registry;
use ferron_observability::baggage::{self, BaggageKeyPromotion, SignalSet};
use ferron_observability::{AccessEvent, AccessVisitor, LogFormatterContext};

use crate::config::LogStyle;
use crate::proto::opentelemetry::proto::common::v1::{AnyValue, KeyValue};
use crate::proto::opentelemetry::proto::logs::v1::LogRecord;

use super::context::{decode_span_id, decode_trace_id};
use super::{any_bool, any_double, any_int, any_string, kv, nanos};

fn format_access_event(
    access_event: &Arc<dyn AccessEvent>,
    log_config: &Arc<ServerConfigurationBlock>,
    registry: &Registry,
) -> Option<String> {
    let formatter_name = log_config
        .get_value("format")
        .and_then(|v| v.as_str())
        .unwrap_or("text");

    // Try to resolve the formatter from the registry
    if let Some(formatter_registry) = registry.get_provider_registry::<LogFormatterContext>() {
        if let Some(formatter) = formatter_registry.get(formatter_name) {
            let mut ctx = LogFormatterContext {
                access_event: access_event.clone(),
                log_config: log_config.clone(),
                output: None,
            };
            if formatter.execute(&mut ctx).is_ok() {
                if let Some(output) = ctx.output {
                    return Some(output);
                }
            }
        }
    }

    None
}

/// Build an OTLP log record from an access event.
///
/// In legacy mode the body is the rendered access log line (via the
/// configured formatter, falling back to `<unknown access log>`). In modern
/// mode the body is a short summary and the traditional fields are mapped
/// onto OTEL semantic-convention attributes.
pub(crate) fn build_access_log_record(
    event: &Arc<dyn AccessEvent>,
    log_config: &Arc<ServerConfigurationBlock>,
    registry: &Registry,
    promotions: &[BaggageKeyPromotion],
    log_style: LogStyle,
    control_plane_metadata: &Option<Arc<BTreeMap<String, String>>>,
    now: SystemTime,
) -> LogRecord {
    let mut attrs: Vec<KeyValue> = Vec::new();
    let (body, event_time) = match log_style {
        LogStyle::Legacy => {
            let body = format_access_event(event, log_config, registry)
                .unwrap_or_else(|| "<unknown access log>".to_string());
            (any_string(body), None)
        }
        LogStyle::Modern => {
            // Map traditional access-log fields onto OTEL semantic-convention
            // attributes. Header fields become `http.request.header.<name>`.
            let mut visitor = OtelAccessAttributeVisitor::default();
            event.visit(&mut visitor);
            for (key, value) in visitor.attributes {
                attrs.push(kv(key, value));
            }
            (
                any_string(format!("Access log ({})", event.protocol())),
                event.event_time(),
            )
        }
    };

    let (mut trace_id, mut span_id, mut flags) = (Vec::new(), Vec::new(), 0u32);
    if let Some(trace_context) = event.trace_context() {
        if let (Some(t), Some(s)) = (
            decode_trace_id(&trace_context.trace_id),
            decode_span_id(&trace_context.span_id),
        ) {
            trace_id = t;
            span_id = s;
            flags = u32::from(trace_context.sampled.unwrap_or(false));
        }
    }

    // Promote configured baggage keys into access log attributes.
    if let Some(baggage_str) = event.trace_context().and_then(|c| c.baggage.as_deref()) {
        let extracted = baggage::extract_promoted_keys(baggage_str, promotions, SignalSet::LOGS);
        for attr in extracted {
            attrs.push(kv(attr.attribute_name, any_string(attr.value)));
        }
    }

    // Inject control plane metadata as access log attributes.
    // Prefer event-level metadata over provider-level metadata.
    let event_metadata = event.control_plane_metadata().map(|m| Arc::new(m.clone()));
    let effective_metadata = event_metadata.as_ref().or(control_plane_metadata.as_ref());
    if let Some(metadata) = effective_metadata {
        for (attr_key, value) in metadata.iter() {
            attrs.push(kv(
                format!("ferron.control_plane.{attr_key}"),
                any_string(value),
            ));
        }
    }

    let observed = nanos(now);
    let time_unix_nano = event_time.map(nanos).unwrap_or(observed);
    LogRecord {
        time_unix_nano,
        observed_time_unix_nano: observed,
        severity_number: 0,
        severity_text: String::new(),
        body: Some(body),
        attributes: attrs,
        dropped_attributes_count: 0,
        flags,
        trace_id,
        span_id,
        event_name: String::new(),
    }
}

/// Captures access-log fields as typed OTEL semantic-convention attributes.
///
/// This visitor drives [`AccessEvent::visit`] and translates the legacy
/// field names (e.g. `client_ip`, `status`, `header_user_agent`) into their
/// OTEL semantic-convention equivalents (e.g. `client.address`,
/// `http.response.status_code`, `http.request.header.user_agent`).
#[derive(Default)]
pub struct OtelAccessAttributeVisitor {
    pub attributes: Vec<(String, AnyValue)>,
}

impl OtelAccessAttributeVisitor {
    fn push(&mut self, key: impl Into<String>, value: AnyValue) {
        self.attributes.push((key.into(), value));
    }
}

impl AccessVisitor for OtelAccessAttributeVisitor {
    fn field_string(&mut self, name: &str, value: &str) {
        match name {
            "path" => self.push("url.path", any_string(value)),
            "path_and_query" => self.push("url.full", any_string(value)),
            "method" => self.push("http.request.method", any_string(value)),
            "version" => self.push("network.protocol.version", any_string(value)),
            "scheme" => self.push("url.scheme", any_string(value)),
            "client_ip_canonical" => self.push("client.address", any_string(value)),
            "server_ip_canonical" => self.push("server.address", any_string(value)),
            "auth_user" => self.push("user.name", any_string(value)),
            "timestamp" | "trace_id" | "span_id" | "client_ip" | "server_ip" => {
                // Drop legacy-only fields; modern telemetry consumers prefer
                // the standard attributes and the record timestamp.
            }
            "content_length" => {
                if let Ok(value) = str::parse::<i64>(value) {
                    self.push("http.response.body.size", any_int(value))
                }
            }
            f if f.contains('.') => {
                self.push(f, any_string(value));
            }
            _ => {
                if let Some(header) = name.strip_prefix("header_") {
                    self.push(format!("http.request.header.{header}"), any_string(value));
                } else {
                    self.push(format!("ferron.custom.{name}"), any_string(value));
                }
            }
        }
    }

    fn field_u64(&mut self, name: &str, value: u64) {
        let int = i64::try_from(value).unwrap_or(i64::MAX);
        match name {
            "status" => self.push("http.response.status_code", any_int(int)),
            "client_port" => self.push("client.port", any_int(int)),
            "server_port" => self.push("server.port", any_int(int)),
            "content_length" => self.push("http.response.body.size", any_int(int)),
            f if f.contains('.') => {
                self.push(f, any_int(int));
            }
            s => self.push(format!("ferron.custom.{s}"), any_int(int)),
        }
    }

    fn field_f64(&mut self, name: &str, value: f64) {
        if name == "duration_secs" {
            self.push("http.server.request.duration", any_double(value));
        } else if name.contains('.') {
            self.push(name, any_double(value));
        } else {
            self.push(format!("ferron.custom.{name}"), any_double(value));
        }
    }

    fn field_bool(&mut self, name: &str, value: bool) {
        if name.contains('.') {
            self.push(name, any_bool(value));
        } else {
            self.push(format!("ferron.custom.{name}"), any_bool(value));
        }
    }
}
