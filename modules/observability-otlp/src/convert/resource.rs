use std::time::{SystemTime, UNIX_EPOCH};

use crate::proto::opentelemetry::proto::common::v1::InstrumentationScope;
use crate::proto::opentelemetry::proto::resource::v1::Resource;

use super::{any_int, any_string, kv};

/// Build the OTLP resource from the service name, including process identity
/// attributes to distinguish between concurrent and sequential process
/// lifetimes.
pub(crate) fn build_resource(service_name: String) -> Resource {
    let pid = std::process::id();
    let start_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    Resource {
        attributes: vec![
            kv("service.name", any_string(service_name)),
            kv("process.pid", any_int(pid as i64)),
            kv("process.start_time", any_int(start_time as i64)),
        ],
        dropped_attributes_count: 0,
        entity_refs: Vec::new(),
    }
}

/// Build the instrumentation scope for a signal (matches the SDK's `"ferron"`
/// and `"ferron.access"` logger and tracer names).
pub(crate) fn build_scope(name: &str) -> InstrumentationScope {
    InstrumentationScope {
        name: name.to_string(),
        version: String::new(),
        attributes: Vec::new(),
        dropped_attributes_count: 0,
    }
}
