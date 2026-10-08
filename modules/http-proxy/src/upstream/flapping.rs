//! Flapping detection for upstream state transitions.
//!
//! When an upstream oscillates rapidly between healthy/unhealthy states,
//! individual transition logs are suppressed and a single flapping notification
//! is emitted instead.

use crate::config::CircuitBreakerConfig;
use crate::types::flapping::{FlappingState, FlappingStateMap};

/// Record a state transition for flapping detection and return whether
/// the upstream is currently flapping.
///
/// This function should be called on every circuit breaker or health check
/// state transition. It pushes the current timestamp into the per-upstream
/// ring buffer, evicts stale entries, and updates the flapping flag.
///
/// Returns `true` if the upstream is now considered flapping (callers
/// should suppress individual transition logs when this returns `true`).
#[inline]
pub fn record_circuit_transition(
    flapping_state_map: Option<&FlappingStateMap>,
    circuit_breaker: &CircuitBreakerConfig,
    upstream: &std::sync::Arc<crate::types::upstream::ResolvedUpstream>,
    event_sink: &ferron_observability::CompositeEventSink,
    event_trace_context: Option<ferron_observability::EventTraceContext>,
) -> bool {
    let Some(flapping_state_map) = flapping_state_map else {
        return false;
    };

    let threshold = circuit_breaker.flapping_transitions;
    if threshold == 0 {
        return false;
    }

    let window = circuit_breaker.flapping_window;

    let state = if let Some(state) = flapping_state_map.get(upstream) {
        if (threshold > 1 && state.threshold().is_none_or(|t| t != threshold))
            || (threshold <= 1 && state.threshold().is_some())
        {
            drop(state);
            let mut state = flapping_state_map
                .entry(upstream.to_owned())
                .or_insert_with(|| FlappingState::with_threshold(threshold));
            state.set_threshold(threshold);
            state.downgrade()
        } else {
            state
        }
    } else {
        flapping_state_map
            .entry(upstream.to_owned())
            .or_insert_with(|| FlappingState::with_threshold(threshold))
            .downgrade()
    };

    let was_flapping = state.is_flapping();
    let is_flapping = state.record_transition(window);

    if is_flapping && !was_flapping {
        let (log_attributes, upstream_id) = crate::upstream::health_upstream_attrs(upstream);
        event_sink.emit(ferron_observability::Event::Log(
            ferron_observability::LogEvent {
                level: ferron_observability::LogLevel::Warn,
                message: format!(
                    "Upstream {} is flapping ({}+ transitions within {:?})",
                    upstream_id, threshold, window
                ),
                summary: "Upstream is flapping".into(),
                target: crate::LOG_TARGET,
                attributes: log_attributes,
                trace_context: event_trace_context.clone(),
            },
        ));
    } else if !is_flapping && was_flapping {
        let (log_attributes, upstream_id) = crate::upstream::health_upstream_attrs(upstream);
        event_sink.emit(ferron_observability::Event::Log(
            ferron_observability::LogEvent {
                level: ferron_observability::LogLevel::Info,
                message: format!(
                    "Upstream {} flapping resolved — transitions have stabilized",
                    upstream_id
                ),
                summary: "Upstream flapping resolved".into(),
                target: crate::LOG_TARGET,
                attributes: log_attributes,
                trace_context: event_trace_context.clone(),
            },
        ));
    }

    is_flapping
}

#[inline]
pub fn emit_flapping_metric(
    event_sink: &ferron_observability::CompositeEventSink,
    upstream: &std::sync::Arc<crate::types::upstream::ResolvedUpstream>,
    value: u64,
    trace_context: Option<ferron_observability::EventTraceContext>,
    metrics_resolved_ip: bool,
) {
    use ferron_observability::{Event, MetricAttributeValue, MetricEvent, MetricType, MetricValue};

    let mut attributes = Vec::with_capacity(4);
    attributes.push((
        "ferron.proxy.backend_url",
        MetricAttributeValue::String(upstream.proxy_to.clone()),
    ));
    if let Some(ref unix_path) = upstream.proxy_unix {
        attributes.push((
            "ferron.proxy.backend_unix_path",
            MetricAttributeValue::String(unix_path.clone()),
        ));
    }
    if metrics_resolved_ip {
        if let Some(ref resolved_ip) = upstream.connect_to {
            attributes.push((
                "ferron.proxy.backend_resolved_ip",
                MetricAttributeValue::String(resolved_ip.to_string()),
            ));
        }
    }
    // Keep the attribute set identical to the one the per-request proxy metrics
    // use, otherwise the same metric name is exported as two distinct series.
    // The DNS outcome has a bounded set of values, so it does not raise
    // cardinality the way the resolved address does.
    attributes.push((
        "ferron.proxy.dns_status",
        MetricAttributeValue::String(upstream.dns_status.as_label().to_string()),
    ));
    event_sink.emit(Event::Metric(MetricEvent {
        name: "ferron.proxy.circuit.flapping",
        attributes,
        ty: MetricType::Gauge,
        value: MetricValue::U64(value),
        unit: Some("{circuit}"),
        description: Some("Whether an upstream backend is flapping (1 = flapping, 0 = stable)."),
        trace_context,
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::flapping::FlappingStateMap;
    use crate::types::upstream::{ResolvedUpstream, UpstreamInner};
    use dashmap::DashMap;
    use rustc_hash::FxBuildHasher;
    use std::sync::Arc;
    use std::time::Duration;

    fn make_upstream(hostname: &str) -> Arc<ResolvedUpstream> {
        Arc::new(ResolvedUpstream {
            proxy_to: hostname.to_string(),
            connect_to: None,
            proxy_unix: None,
            inner: UpstreamInner {
                weight: 0,
                mtls: None,
                priority: 0,
                connection_timeout: None,
                idle_timeout: Duration::MAX,
                limit: None,
            },
            dns_status: crate::types::upstream::DnsResolutionStatus::NotApplicable,
        })
    }

    #[test]
    fn test_no_flapping_below_threshold() {
        let state_map: FlappingStateMap = Arc::new(DashMap::with_hasher(FxBuildHasher));
        let cb = CircuitBreakerConfig {
            flapping_transitions: 3,
            flapping_window: Duration::from_secs(10),
            ..Default::default()
        };
        let event_sink = ferron_observability::CompositeEventSink::new(vec![]);

        assert!(!record_circuit_transition(
            Some(&state_map),
            &cb,
            &make_upstream("http://localhost:8080"),
            &event_sink,
            None,
        ));
        assert!(!record_circuit_transition(
            Some(&state_map),
            &cb,
            &make_upstream("http://localhost:8080"),
            &event_sink,
            None,
        ));
        // 2 transitions < threshold of 3 -> not flapping
    }

    #[test]
    fn test_flapping_at_threshold() {
        let state_map: FlappingStateMap = Arc::new(DashMap::with_hasher(FxBuildHasher));
        let cb = CircuitBreakerConfig {
            flapping_transitions: 3,
            flapping_window: Duration::from_secs(10),
            ..Default::default()
        };
        let event_sink = ferron_observability::CompositeEventSink::new(vec![]);

        record_circuit_transition(
            Some(&state_map),
            &cb,
            &make_upstream("http://localhost:8080"),
            &event_sink,
            None,
        );
        record_circuit_transition(
            Some(&state_map),
            &cb,
            &make_upstream("http://localhost:8080"),
            &event_sink,
            None,
        );
        // 3rd transition reaches threshold -> flapping
        assert!(record_circuit_transition(
            Some(&state_map),
            &cb,
            &make_upstream("http://localhost:8080"),
            &event_sink,
            None,
        ));
    }

    #[test]
    fn test_flapping_resolves_after_window() {
        let state_map: FlappingStateMap = Arc::new(DashMap::with_hasher(FxBuildHasher));
        let cb = CircuitBreakerConfig {
            flapping_transitions: 2,
            flapping_window: Duration::from_millis(50),
            ..Default::default()
        };
        let event_sink = ferron_observability::CompositeEventSink::new(vec![]);

        // Trigger flapping
        record_circuit_transition(
            Some(&state_map),
            &cb,
            &make_upstream("http://localhost:8080"),
            &event_sink,
            None,
        );
        assert!(record_circuit_transition(
            Some(&state_map),
            &cb,
            &make_upstream("http://localhost:8080"),
            &event_sink,
            None,
        ));

        // Wait for window to expire
        std::thread::sleep(Duration::from_millis(60));

        // New transition after window -> transitions evicted -? not flapping
        assert!(!record_circuit_transition(
            Some(&state_map),
            &cb,
            &make_upstream("http://localhost:8080"),
            &event_sink,
            None,
        ));
    }

    #[test]
    fn test_zero_threshold_disables() {
        let state_map: FlappingStateMap = Arc::new(DashMap::with_hasher(FxBuildHasher));
        let cb = CircuitBreakerConfig {
            flapping_transitions: 0,
            flapping_window: Duration::from_secs(10),
            ..Default::default()
        };
        let event_sink = ferron_observability::CompositeEventSink::new(vec![]);

        assert!(!record_circuit_transition(
            Some(&state_map),
            &cb,
            &make_upstream("http://localhost:8080"),
            &event_sink,
            None,
        ));
    }

    #[test]
    fn test_none_map_returns_false() {
        let cb = CircuitBreakerConfig::default();
        let event_sink = ferron_observability::CompositeEventSink::new(vec![]);

        assert!(!record_circuit_transition(
            None,
            &cb,
            &make_upstream("http://localhost:8080"),
            &event_sink,
            None,
        ));
    }
}
