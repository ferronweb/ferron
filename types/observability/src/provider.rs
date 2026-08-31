use std::collections::BTreeMap;
use std::sync::Arc;

use crate::{AccessEvent, EventSink, LogEvent};
use ferron_core::config::ServerConfigurationBlock;

/// Context passed to observability [`Provider`](ferron_core::providers::Provider) implementations.
///
/// A provider reads [`log_config`](ObservabilityContext::log_config) to
/// obtain backend-specific settings (file path, format, OTLP endpoint, etc.)
/// and sets [`sink`](ObservabilityContext::sink) to an initialized
/// [`EventSink`].
pub struct ObservabilityContext {
    /// The observability configuration block for this backend.
    pub log_config: Arc<ServerConfigurationBlock>,
    /// The initialized event sink, set by the provider during execute.
    pub sink: Option<Arc<dyn EventSink>>,
    /// Control plane metadata to include in observability signals
    /// (from `control_plane { metadata { ... } }`).
    pub control_plane_metadata: Option<Arc<BTreeMap<String, String>>>,
}

impl ObservabilityContext {
    /// Create a new context with the given configuration block.
    ///
    /// The `sink` and `control_plane_metadata` fields are left unset
    /// and should be populated by the provider during `execute`.
    pub fn new(log_config: Arc<ServerConfigurationBlock>) -> Self {
        Self {
            log_config,
            sink: None,
            control_plane_metadata: None,
        }
    }
}

/// Context passed to access log formatters.
///
/// A formatter receives an [`AccessEvent`] and the configuration block,
/// then sets [`output`](LogFormatterContext::output) to the formatted
/// log line.
pub struct LogFormatterContext {
    /// The access event to format.
    pub access_event: Arc<dyn AccessEvent>,
    /// The observability configuration block.
    pub log_config: Arc<ServerConfigurationBlock>,
    /// The formatted output. Set this in the formatter.
    pub output: Option<String>,
}

/// Context passed to application log formatters.
///
/// A formatter receives a [`LogEvent`] and the configuration block,
/// then sets [`output`](ApplicationLogFormatterContext::output) to the
/// formatted log line.
pub struct ApplicationLogFormatterContext<'a> {
    /// The application log event to format.
    pub log_event: &'a LogEvent,
    /// The observability configuration block.
    pub log_config: Arc<ServerConfigurationBlock>,
    /// The formatted output. Set this in the formatter.
    pub output: Option<String>,
}
