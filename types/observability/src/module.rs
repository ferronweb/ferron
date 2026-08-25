use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Once};

use crate::{AccessEvent, Event};

/// Wrapper that carries an event with its configuration through the channel.
/// Shared by all observability backend modules (console, file, prometheus,
/// statsd, otlp).
pub struct ConfiguredEvent {
    pub event: Arc<Event>,
    pub log_config: Arc<ferron_core::config::ServerConfigurationBlock>,
    pub control_plane_metadata: Option<Arc<BTreeMap<String, String>>>,
}

/// A minimal [`AccessEvent`] implementation used as a placeholder during
/// provider initialization. Prometheus and StatsD providers send this fake
/// event to capture the initial log configuration without requiring a real
/// access event.
pub struct InitAccessEvent;

impl AccessEvent for InitAccessEvent {
    fn protocol(&self) -> &'static str {
        "init"
    }

    fn visit(&self, _visitor: &mut dyn crate::AccessVisitor) {}
}

/// Once guard shared by all observability backends for warn-once dropped-event
/// logging. The warning text includes the backend name, so the single guard is
/// sufficient.
static DROPPED_EVENT: Once = Once::new();

/// Send a configured event through the channel, updating queue-length and
/// dropped-event metrics on success and failure respectively.
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

/// Format control-plane metadata as a `[k=v ...] ` prefix string for log lines.
pub fn format_metadata_prefix(metadata: Option<&Arc<BTreeMap<String, String>>>) -> String {
    match metadata {
        Some(meta) if !meta.is_empty() => {
            let parts: Vec<String> = meta.iter().map(|(k, v)| format!("{}={}", k, v)).collect();
            format!("[{}] ", parts.join(" "))
        }
        _ => String::new(),
    }
}
