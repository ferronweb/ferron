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
pub fn record_circuit_transition(
    flapping_state_map: Option<&FlappingStateMap>,
    circuit_breaker: &CircuitBreakerConfig,
    upstream_url: &str,
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

    let state = if let Some(state) = flapping_state_map.get(upstream_url) {
        if (threshold > 1 && state.threshold().is_none_or(|t| t != threshold))
            || (threshold <= 1 && state.threshold().is_some())
        {
            drop(state);
            let mut state = flapping_state_map
                .entry(upstream_url.to_string())
                .or_insert_with(|| FlappingState::with_threshold(threshold));
            state.set_threshold(threshold);
            state.downgrade()
        } else {
            state
        }
    } else {
        flapping_state_map
            .entry(upstream_url.to_string())
            .or_insert_with(|| FlappingState::with_threshold(threshold))
            .downgrade()
    };

    let was_flapping = state.is_flapping();
    let is_flapping = state.record_transition(window);

    if is_flapping && !was_flapping {
        // Flapping just started — emit notification
        event_sink.emit(ferron_observability::Event::Log(
            ferron_observability::LogEvent {
                level: ferron_observability::LogLevel::Warn,
                message: format!(
                    "Upstream {} is flapping ({}+ transitions within {:?})",
                    upstream_url, threshold, window
                ),
                summary: "Upstream is flapping".into(),
                target: crate::LOG_TARGET,
                attributes: vec![(
                    "upstream.address",
                    ferron_observability::LogAttributeValue::String(upstream_url.to_string()),
                )],
                trace_context: event_trace_context.clone(),
            },
        ));
        emit_flapping_metric(event_sink, upstream_url, 1, event_trace_context);
    } else if !is_flapping && was_flapping {
        // Flapping resolved — emit recovery notification
        event_sink.emit(ferron_observability::Event::Log(
            ferron_observability::LogEvent {
                level: ferron_observability::LogLevel::Info,
                message: format!(
                    "Upstream {} flapping resolved — transitions have stabilized",
                    upstream_url
                ),
                summary: "Upstream flapping resolved".into(),
                target: crate::LOG_TARGET,
                attributes: vec![(
                    "upstream.address",
                    ferron_observability::LogAttributeValue::String(upstream_url.to_string()),
                )],
                trace_context: event_trace_context.clone(),
            },
        ));
        emit_flapping_metric(event_sink, upstream_url, 0, event_trace_context);
    } else if is_flapping {
        // Still flapping — just update the metric on each transition
        emit_flapping_metric(event_sink, upstream_url, 1, event_trace_context);
    }

    is_flapping
}

fn emit_flapping_metric(
    event_sink: &ferron_observability::CompositeEventSink,
    upstream_url: &str,
    value: u64,
    trace_context: Option<ferron_observability::EventTraceContext>,
) {
    use ferron_observability::{Event, MetricAttributeValue, MetricEvent, MetricType, MetricValue};

    let attributes = vec![(
        "ferron.proxy.backend_url",
        MetricAttributeValue::String(upstream_url.to_string()),
    )];
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
    use dashmap::DashMap;
    use rustc_hash::FxBuildHasher;
    use std::sync::Arc;
    use std::time::Duration;

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
            "http://localhost:8080",
            &event_sink,
            None,
        ));
        assert!(!record_circuit_transition(
            Some(&state_map),
            &cb,
            "http://localhost:8080",
            &event_sink,
            None,
        ));
        // 2 transitions < threshold of 3 → not flapping
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
            "http://localhost:8080",
            &event_sink,
            None,
        );
        record_circuit_transition(
            Some(&state_map),
            &cb,
            "http://localhost:8080",
            &event_sink,
            None,
        );
        // 3rd transition reaches threshold → flapping
        assert!(record_circuit_transition(
            Some(&state_map),
            &cb,
            "http://localhost:8080",
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
            "http://localhost:8080",
            &event_sink,
            None,
        );
        assert!(record_circuit_transition(
            Some(&state_map),
            &cb,
            "http://localhost:8080",
            &event_sink,
            None,
        ));

        // Wait for window to expire
        std::thread::sleep(Duration::from_millis(60));

        // New transition after window → transitions evicted → not flapping
        assert!(!record_circuit_transition(
            Some(&state_map),
            &cb,
            "http://localhost:8080",
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
            "http://localhost:8080",
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
            "http://localhost:8080",
            &event_sink,
            None,
        ));
    }
}
