//! Types used by observability sink modules.

use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Once};

use crate::{AccessEvent, Event};

/// An event bundled with its configuration for channel transport.
///
/// Observability backend modules (console, file, Prometheus, StatsD, OTLP)
/// receive events through an `async_channel`. This wrapper carries the event
/// along with the configuration block and control plane metadata needed
/// to format and emit it.
pub struct ConfiguredEvent {
    /// The observability event.
    pub event: Arc<Event>,
    /// The observability configuration block for the target backend.
    pub log_config: Arc<ferron_core::config::ServerConfigurationBlock>,
    /// Control plane metadata to include as attributes.
    pub control_plane_metadata: Option<Arc<BTreeMap<String, String>>>,
}

/// A no-op [`AccessEvent`] used during provider initialization.
///
/// Prometheus and StatsD providers send this fake event to capture the
/// initial log configuration without requiring a real access event.
pub struct InitAccessEvent;

impl AccessEvent for InitAccessEvent {
    fn protocol(&self) -> &'static str {
        "init"
    }

    fn visit(&self, _visitor: &mut dyn crate::AccessVisitor) {}
}

/// Capacity of the bounded event channel for observability backends.
///
/// Each queue slot holds a `ConfiguredEvent` envelope; the events themselves
/// are reference-counted so payloads are shared between sender and receiver.
pub const EVENT_CHANNEL_CAPACITY: usize = 131072;

/// Lazily-initialized bounded event channel shared between an observability
/// backend's provider factory and its background module.
///
/// Module loaders are constructed for every backend in `default_profile()`
/// before the configuration is known, so allocating the channel eagerly in
/// `Default` wastes ~4 MB per backend when it is never configured. Holding
/// this shared cell instead defers the allocation until the backend is
/// actually selected by the configuration.
pub struct SharedEventChannel {
    cell: std::sync::OnceLock<(
        async_channel::Sender<ConfiguredEvent>,
        async_channel::Receiver<ConfiguredEvent>,
    )>,
}

impl SharedEventChannel {
    /// Create an empty (unallocated) channel cell.
    pub fn new() -> Self {
        Self {
            cell: std::sync::OnceLock::new(),
        }
    }

    /// Get the channel, allocating it on first use.
    pub fn get_or_init(
        &self,
    ) -> &(
        async_channel::Sender<ConfiguredEvent>,
        async_channel::Receiver<ConfiguredEvent>,
    ) {
        self.cell
            .get_or_init(|| async_channel::bounded(EVENT_CHANNEL_CAPACITY))
    }
}

impl Default for SharedEventChannel {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns true if any observability block in the global config or any
/// per-host port block selects `provider_name`.
///
/// This covers both explicit `observability { provider <name>; ... }` blocks
/// and alias directives (`log`/`error_log` map to `file`, `console_log`
/// maps to `console`).
pub fn config_uses_provider(
    config: &ferron_core::config::ServerConfiguration,
    provider_name: &str,
) -> bool {
    use crate::config::ObservabilityConfigExtractor;

    if let Ok(blocks) =
        ObservabilityConfigExtractor::new(&config.global_config).extract_observability_blocks()
    {
        if blocks.iter().any(|block| {
            block.get_value("provider").and_then(|v| v.as_str()) == Some(provider_name)
        }) {
            return true;
        }
    }

    for ports in config.ports.values() {
        for port in ports {
            for (_, host_block) in &port.hosts {
                if let Ok(blocks) =
                    ObservabilityConfigExtractor::new(host_block).extract_observability_blocks()
                {
                    if blocks.iter().any(|block| {
                        block.get_value("provider").and_then(|v| v.as_str()) == Some(provider_name)
                    }) {
                        return true;
                    }
                }
            }
        }
    }

    false
}

/// Once guard shared by all observability backends for warn-once dropped-event
/// logging. The warning text includes the backend name, so the single guard is
/// sufficient.
static DROPPED_EVENT: Once = Once::new();

/// Send an event through the channel, updating queue and drop metrics.
///
/// If the channel is full, the event is dropped and a warn-once log is emitted.
pub fn try_send_event(
    sender: &async_channel::Sender<ConfiguredEvent>,
    event: Arc<Event>,
    log_config: &Arc<ferron_core::config::ServerConfigurationBlock>,
    control_plane_metadata: &Option<Arc<BTreeMap<String, String>>>,
    backend_name: &str,
) {
    match sender.try_send(ConfiguredEvent {
        event,
        log_config: log_config.clone(),
        control_plane_metadata: control_plane_metadata.clone(),
    }) {
        Ok(_) => {
            ferron_core::admin::ADMIN_METRICS
                .observability_event_queue_len
                .fetch_add(1, Ordering::Relaxed);
        }
        Err(_) => {
            ferron_core::admin::ADMIN_METRICS
                .observability_events_dropped
                .fetch_add(1, Ordering::Relaxed);

            DROPPED_EVENT.call_once(|| {
                ferron_core::log_warn!(
                    "Observability event dropped (`{}` observability backend). \
                     This may be caused by high server load.",
                    backend_name
                );
            });
        }
    }
}

/// Format control-plane metadata as a `[k=v ...] ` prefix for log lines.
///
/// Returns an empty string when there is no metadata.
pub fn format_metadata_prefix(metadata: Option<&Arc<BTreeMap<String, String>>>) -> String {
    match metadata {
        Some(meta) if !meta.is_empty() => {
            let parts: Vec<String> = meta.iter().map(|(k, v)| format!("{}={}", k, v)).collect();
            format!("[{}] ", parts.join(" "))
        }
        _ => String::new(),
    }
}
