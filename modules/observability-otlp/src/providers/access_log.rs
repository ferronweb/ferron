use std::collections::BTreeMap;
use std::sync::Arc;

use ferron_core::config::ServerConfigurationBlock;
use ferron_core::registry::Registry;
use ferron_observability::baggage::{self, BaggageKeyPromotion, SignalSet};
use ferron_observability::{AccessEvent, AccessVisitor, LogFormatterContext};
use opentelemetry::logs::AnyValue;

use crate::config::LogStyle;

use super::context::trace_flags;

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

pub(crate) fn emit_access_log(
    provider: &opentelemetry_sdk::logs::SdkLoggerProvider,
    event: &Arc<dyn AccessEvent>,
    log_config: &Arc<ServerConfigurationBlock>,
    registry: &Registry,
    promotions: &[BaggageKeyPromotion],
    log_style: LogStyle,
    control_plane_metadata: &Option<Arc<BTreeMap<String, String>>>,
) {
    use opentelemetry::logs::{LogRecord, Logger, LoggerProvider};

    let logger = provider.logger("ferron.access");
    let mut record = logger.create_log_record();
    match log_style {
        LogStyle::Legacy => {
            if let Some(body) = format_access_event(event, log_config, registry) {
                record.set_body(AnyValue::String(body.into()));
            } else {
                record.set_body(AnyValue::String("<unknown access log>".into()));
            }
        }
        LogStyle::Modern => {
            record.set_body(AnyValue::String(
                format!("Access log ({})", event.protocol()).into(),
            ));
            // Set timestamp from the access event when available
            if let Some(time) = event.event_time() {
                record.set_timestamp(time);
            }
            // Map traditional access-log fields onto OTEL semantic-convention
            // attributes. Header fields become `http.request.header.<name>`.
            let mut visitor = OtelAccessAttributeVisitor::default();
            event.visit(&mut visitor);
            for (key, value) in visitor.attributes {
                record.add_attribute(key, value);
            }
        }
    }
    if let Some(trace_context) = event.trace_context() {
        if let (Ok(trace_id_str), Ok(span_id_str)) = (
            std::str::from_utf8(&trace_context.trace_id),
            std::str::from_utf8(&trace_context.span_id),
        ) {
            if let (Ok(trace_id), Ok(span_id)) = (
                opentelemetry::TraceId::from_hex(trace_id_str),
                opentelemetry::SpanId::from_hex(span_id_str),
            ) {
                record.set_trace_context(trace_id, span_id, trace_flags(trace_context.sampled));
            }
        }
    }

    // Promote configured baggage keys into access log attributes
    if let Some(baggage_str) = event.trace_context().and_then(|c| c.baggage.as_deref()) {
        let extracted = baggage::extract_promoted_keys(baggage_str, promotions, SignalSet::LOGS);
        for attr in extracted {
            record.add_attribute(attr.attribute_name, AnyValue::String(attr.value.into()));
        }
    }

    // Inject control plane metadata as access log attributes
    // Prefer event-level metadata over provider-level metadata
    let event_metadata = event.control_plane_metadata().map(|m| Arc::new(m.clone()));
    let effective_metadata = event_metadata.as_ref().or(control_plane_metadata.as_ref());
    if let Some(metadata) = effective_metadata {
        for (key, value) in metadata.iter() {
            let attr_key = format!("ferron.control_plane.{}", key);
            record.add_attribute(attr_key, AnyValue::String(value.clone().into()));
        }
    }

    logger.emit(record);
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
            "path" => self.push("url.path", AnyValue::String(value.to_string().into())),
            "path_and_query" => self.push("url.full", AnyValue::String(value.to_string().into())),
            "method" => self.push(
                "http.request.method",
                AnyValue::String(value.to_string().into()),
            ),
            "version" => self.push(
                "network.protocol.version",
                AnyValue::String(value.to_string().into()),
            ),
            "scheme" => self.push("url.scheme", AnyValue::String(value.to_string().into())),
            "client_ip_canonical" => {
                self.push("client.address", AnyValue::String(value.to_string().into()))
            }
            "server_ip_canonical" => {
                self.push("server.address", AnyValue::String(value.to_string().into()))
            }
            "auth_user" => self.push("user.name", AnyValue::String(value.to_string().into())),
            "timestamp" | "trace_id" | "span_id" | "client_ip" | "server_ip" => {
                // Drop legacy-only fields; modern telemetry consumers prefer the
                // standard attributes and the record timestamp.
            }
            "content_length" => {
                if let Ok(value) = str::parse::<i64>(value) {
                    self.push("http.response.body.size", AnyValue::Int(value))
                }
            }
            f if f.contains(".") => {
                self.push(f, AnyValue::String(value.to_string().into()));
            }
            _ => {
                if let Some(header) = name.strip_prefix("header_") {
                    self.push(
                        format!("http.request.header.{}", header),
                        AnyValue::String(value.to_string().into()),
                    );
                } else {
                    self.push(
                        format!("ferron.custom.{name}"),
                        AnyValue::String(value.to_string().into()),
                    );
                }
            }
        }
    }

    fn field_u64(&mut self, name: &str, value: u64) {
        match name {
            "status" => self.push(
                "http.response.status_code",
                AnyValue::Int(i64::try_from(value).unwrap_or(i64::MAX)),
            ),
            "client_port" => self.push(
                "client.port",
                AnyValue::Int(i64::try_from(value).unwrap_or(i64::MAX)),
            ),
            "server_port" => self.push(
                "server.port",
                AnyValue::Int(i64::try_from(value).unwrap_or(i64::MAX)),
            ),
            "content_length" => self.push(
                "http.response.body.size",
                AnyValue::Int(i64::try_from(value).unwrap_or(i64::MAX)),
            ),
            f if f.contains(".") => {
                self.push(f, AnyValue::Int(i64::try_from(value).unwrap_or(i64::MAX)));
            }
            s => self.push(
                format!("ferron.custom.{s}"),
                AnyValue::Int(i64::try_from(value).unwrap_or(i64::MAX)),
            ),
        }
    }

    fn field_f64(&mut self, name: &str, value: f64) {
        if name == "duration_secs" {
            self.push("http.server.request.duration", AnyValue::Double(value));
        } else if name.contains(".") {
            self.push(name, AnyValue::Double(value));
        } else {
            self.push(format!("ferron.custom.{name}"), AnyValue::Double(value));
        }
    }

    fn field_bool(&mut self, name: &str, value: bool) {
        if name.contains(".") {
            self.push(name, AnyValue::Boolean(value));
        } else {
            self.push(format!("ferron.custom.{name}"), AnyValue::Boolean(value));
        }
    }
}
