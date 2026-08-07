use opentelemetry::KeyValue;
use opentelemetry_sdk::Resource;

/// Build an OTLP resource from the service name, including process identity
/// attributes to distinguish between concurrent and sequential process lifetimes.
pub(crate) fn build_resource(service_name: String) -> Resource {
    let pid = std::process::id();
    let start_time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    Resource::builder()
        .with_service_name(service_name)
        .with_attribute(KeyValue::new("process.pid", pid as i64))
        .with_attribute(KeyValue::new("process.start_time", start_time as i64))
        .build()
}
