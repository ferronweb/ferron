//! Configuration parsing for the `json_errors` directive.

use ferron_core::config::layer::LayeredConfiguration;

/// Output format for JSON error responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JsonErrorFormat {
    /// RFC 9457 `application/problem+json` with `type`, `title`, `status`, `detail` fields.
    #[default]
    Problem,
    /// Simple `application/json` with `error`, `status`, `detail` fields.
    Simple,
}

/// Parsed configuration for the `json_errors` directive.
pub struct JsonErrorConfig {
    /// Whether JSON error responses are enabled.
    pub enabled: bool,
    /// Output format (RFC 9457 problem details or simple JSON).
    pub format: JsonErrorFormat,
    /// Base URI for the `type` field in RFC 9457 format. `{status}` is replaced with the HTTP status code.
    /// Default: `about:blank`
    pub type_uri: String,
    /// Whether to include `trace_id` when trace context is available.
    pub trace_id: bool,
}

impl Default for JsonErrorConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            format: JsonErrorFormat::default(),
            type_uri: "about:blank".to_string(),
            trace_id: true,
        }
    }
}

impl JsonErrorConfig {
    /// Parse `json_errors` configuration from the layered configuration.
    pub fn from_config(config: &LayeredConfiguration) -> Self {
        let mut result = Self::default();

        let entries = config.get_entries("json_errors", true);

        for entry in &entries {
            // get_flag() returns true for bare directives, or the boolean value for "json_errors true/false"
            result.enabled = entry.get_flag();

            if let Some(children) = &entry.children {
                if let Some(val) = children.get_value("format").and_then(|v| v.as_str()) {
                    match val {
                        "problem" => result.format = JsonErrorFormat::Problem,
                        "simple" => result.format = JsonErrorFormat::Simple,
                        _ => {}
                    }
                }

                if let Some(val) = children.get_value("type_uri").and_then(|v| v.as_str()) {
                    result.type_uri = val.to_string();
                }

                // Parse `trace_id` subdirective (default: true)
                if let Some(val) = children.get_value("trace_id") {
                    if let Some(b) = val.as_boolean() {
                        result.trace_id = b;
                    }
                }
            }
        }

        result
    }
}
