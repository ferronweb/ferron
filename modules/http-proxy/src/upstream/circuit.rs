//! Circuit breaker implementation for protecting upstream backends.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::config::CircuitBreakerConfig;
use crate::types::circuit::{
    CircuitBreakerStateMap, CIRCUIT_BREAKER_STATUS_CLOSED, CIRCUIT_BREAKER_STATUS_HALFOPEN,
    CIRCUIT_BREAKER_STATUS_OPEN,
};
use crate::types::upstream::UpstreamInner;

/// Returns whether a backend is currently available for new circuit-breaker traffic.
pub fn is_circuit_breaker_available(
    circuit_breaker_state: Option<&CircuitBreakerStateMap>,
    circuit_breaker: &CircuitBreakerConfig,
    upstream: &Arc<UpstreamInner>,
) -> bool {
    if !circuit_breaker.enabled {
        return true;
    }

    let Some(circuit_breaker_state) = circuit_breaker_state else {
        return true;
    };

    let Some(state) = circuit_breaker_state.get(upstream) else {
        return true;
    };

    match state.status.load(Ordering::Relaxed) {
        CIRCUIT_BREAKER_STATUS_CLOSED => true,
        CIRCUIT_BREAKER_STATUS_OPEN => state
            .opened_at
            .read()
            .is_some_and(|opened_at| opened_at.elapsed() >= circuit_breaker.open_duration),
        CIRCUIT_BREAKER_STATUS_HALFOPEN => !state.half_open_in_flight.load(Ordering::Relaxed),
        _ => false,
    }
}

/// Record a transport-level backend failure.
#[allow(clippy::too_many_arguments)]
#[inline]
pub fn record_backend_transport_failure(
    circuit_breaker_state: Option<&CircuitBreakerStateMap>,
    circuit_breaker: &CircuitBreakerConfig,
    upstream: &Arc<UpstreamInner>,
    metrics: &mut crate::ProxyMetrics,
    event_sink: &ferron_observability::CompositeEventSink,
    event_trace_context: Option<ferron_observability::EventTraceContext>,
) {
    if record_circuit_breaker_failure(
        circuit_breaker_state,
        circuit_breaker,
        upstream,
        event_sink,
        event_trace_context,
    ) {
        metrics
            .circuit_breaker_unhealthy_backends
            .push(upstream.clone());
    }
}

/// Record an upstream response for the circuit breaker state machine.
#[inline]
pub fn record_backend_response(
    circuit_breaker_state: Option<&CircuitBreakerStateMap>,
    circuit_breaker: &CircuitBreakerConfig,
    upstream: &Arc<UpstreamInner>,
    status: u16,
    metrics: &mut crate::ProxyMetrics,
    event_sink: &ferron_observability::CompositeEventSink,
    trace_context: Option<ferron_observability::EventTraceContext>,
) {
    let should_open = if circuit_breaker.record_5xx && is_circuit_breaker_failure_status(status) {
        record_circuit_breaker_failure(
            circuit_breaker_state,
            circuit_breaker,
            upstream,
            event_sink,
            trace_context,
        )
    } else {
        record_circuit_breaker_success(
            circuit_breaker_state,
            circuit_breaker,
            upstream,
            event_sink,
            trace_context,
        );
        false
    };

    if should_open {
        metrics
            .circuit_breaker_unhealthy_backends
            .push(upstream.clone());
    }
}

/// Record a circuit breaker slot acquisition attempt.
pub fn try_acquire_circuit_breaker_slot(
    circuit_breaker_state: Option<&CircuitBreakerStateMap>,
    circuit_breaker: &CircuitBreakerConfig,
    upstream: &Arc<UpstreamInner>,
    event_sink: &ferron_observability::CompositeEventSink,
    event_trace_context: Option<ferron_observability::EventTraceContext>,
) -> bool {
    if !circuit_breaker.enabled {
        return true;
    }

    let Some(circuit_breaker_state) = circuit_breaker_state else {
        return true;
    };

    // Get a reference instead of a mutable reference for fast paths.
    let state = if let Some(state) = circuit_breaker_state.get(upstream) {
        state
    } else {
        let mut state = circuit_breaker_state.entry(upstream.clone()).or_default();
        state.recent_failures = (circuit_breaker.max_fails > 1).then(|| {
            Arc::new(crossbeam_queue::ArrayQueue::new(
                (circuit_breaker.max_fails as usize).saturating_sub(1),
            ))
        });
        state.downgrade()
    };

    match state.status.load(Ordering::Relaxed) {
        CIRCUIT_BREAKER_STATUS_CLOSED => true,
        CIRCUIT_BREAKER_STATUS_OPEN => {
            {
                let mut opened_at_ref = state.opened_at.upgradable_read();
                let opened_at = if let Some(opened_at) = &*opened_at_ref {
                    opened_at
                } else {
                    // This might be a rare edge case, reset the opened_at counter...
                    opened_at_ref.with_upgraded(|r| {
                        if r.is_some() {
                            // Double check, to not overwrite the instant
                            return;
                        }
                        *r = Some(std::time::Instant::now());
                    });
                    let Some(opened_at) = &*opened_at_ref else {
                        // At this point, something else has overwriten the value, so return `false`
                        return false;
                    };
                    opened_at
                };

                if opened_at.elapsed() < circuit_breaker.open_duration {
                    return false;
                }
            }

            state
                .status
                .store(CIRCUIT_BREAKER_STATUS_HALFOPEN, Ordering::Relaxed);
            *state.opened_at.write() = None;
            state.half_open_in_flight.store(true, Ordering::Relaxed);
            state.half_open_pass_count.store(0, Ordering::Relaxed);
            event_sink.emit(ferron_observability::Event::Log(
                ferron_observability::LogEvent {
                    level: ferron_observability::LogLevel::Info,
                    message: format!(
                        "Upstream {} circuit transitioned to half-open",
                        upstream.proxy_to
                    ),
                    summary: "Upstream circuit transitioned to half-open".into(),
                    target: crate::LOG_TARGET,
                    attributes: vec![(
                        "upstream.address",
                        ferron_observability::LogAttributeValue::String(upstream.proxy_to.clone()),
                    )],
                    trace_context: event_trace_context.clone(),
                },
            ));
            emit_circuit_metric(
                event_sink,
                upstream,
                "ferron.proxy.circuit.state",
                ferron_observability::MetricType::Gauge,
                ferron_observability::MetricValue::U64(1), // HalfOpen = 1
                event_trace_context.clone(),
            );
            emit_circuit_metric(
                event_sink,
                upstream,
                "ferron.proxy.circuit.open_total",
                ferron_observability::MetricType::Counter,
                ferron_observability::MetricValue::I64(-1),
                event_trace_context,
            );
            true
        }
        CIRCUIT_BREAKER_STATUS_HALFOPEN => !state.half_open_in_flight.swap(true, Ordering::Relaxed),
        _ => false, // Possibly corrupted state
    }
}

fn emit_circuit_metric(
    event_sink: &ferron_observability::CompositeEventSink,
    upstream: &Arc<UpstreamInner>,
    name: &'static str,
    metric_type: ferron_observability::MetricType,
    value: ferron_observability::MetricValue,
    trace_context: Option<ferron_observability::EventTraceContext>,
) {
    use ferron_observability::{Event, MetricAttributeValue, MetricEvent};
    event_sink.emit(Event::Metric(MetricEvent {
        name,
        attributes: vec![(
            "ferron.proxy.backend_url",
            MetricAttributeValue::String(upstream.proxy_to.clone()),
        )],
        ty: metric_type,
        value,
        unit: Some("{circuit}"),
        description: Some("Circuit breaker state and transitions for upstream backends."),
        trace_context,
    }));
}

fn record_circuit_breaker_failure(
    circuit_breaker_state: Option<&CircuitBreakerStateMap>,
    circuit_breaker: &CircuitBreakerConfig,
    upstream: &Arc<UpstreamInner>,
    event_sink: &ferron_observability::CompositeEventSink,
    event_trace_context: Option<ferron_observability::EventTraceContext>,
) -> bool {
    if !circuit_breaker.enabled {
        return false;
    }

    let Some(circuit_breaker_state) = circuit_breaker_state else {
        return false;
    };

    let now = std::time::Instant::now();
    let state = if let Some(state) = circuit_breaker_state.get(upstream) {
        state
    } else {
        let mut state = circuit_breaker_state.entry(upstream.clone()).or_default();
        state.recent_failures = (circuit_breaker.max_fails > 1).then(|| {
            Arc::new(crossbeam_queue::ArrayQueue::new(
                (circuit_breaker.max_fails as usize).saturating_sub(1),
            ))
        });
        state.downgrade()
    };

    match state.status.load(Ordering::Relaxed) {
        CIRCUIT_BREAKER_STATUS_HALFOPEN => {
            state
                .status
                .store(CIRCUIT_BREAKER_STATUS_OPEN, Ordering::Relaxed);
            state.half_open_in_flight.store(false, Ordering::Relaxed);
            state.half_open_pass_count.store(0, Ordering::Relaxed);
            if let Some(rf) = &state.recent_failures {
                while rf.pop().is_some() {}
            }
            *state.opened_at.write() = Some(now);
            event_sink.emit(ferron_observability::Event::Log(
                ferron_observability::LogEvent {
                    level: ferron_observability::LogLevel::Warn,
                    message: format!(
                        "Upstream {} circuit reopened after a half-open trial failure",
                        upstream.proxy_to
                    ),
                    summary: "Upstream circuit reopened after half-open trial failure".into(),
                    target: crate::LOG_TARGET,
                    attributes: vec![(
                        "upstream.address",
                        ferron_observability::LogAttributeValue::String(upstream.proxy_to.clone()),
                    )],
                    trace_context: event_trace_context.clone(),
                },
            ));
            emit_circuit_metric(
                event_sink,
                upstream,
                "ferron.proxy.circuit.state",
                ferron_observability::MetricType::Gauge,
                ferron_observability::MetricValue::U64(2), // Open = 2
                event_trace_context.clone(),
            );
            emit_circuit_metric(
                event_sink,
                upstream,
                "ferron.proxy.circuit.open_total",
                ferron_observability::MetricType::Counter,
                ferron_observability::MetricValue::I64(1),
                event_trace_context,
            );
            true
        }
        CIRCUIT_BREAKER_STATUS_OPEN => {
            *state.opened_at.write() = Some(now);
            false
        }
        CIRCUIT_BREAKER_STATUS_CLOSED
            if state.recent_failures.as_ref().is_none_or(|rf| {
                rf.force_push(now)
                    .is_some_and(|timestamp| now.duration_since(timestamp) < circuit_breaker.window)
            }) =>
        {
            state
                .status
                .store(CIRCUIT_BREAKER_STATUS_OPEN, Ordering::Relaxed);

            if let Some(rf) = &state.recent_failures {
                while rf.pop().is_some() {}
            }
            *state.opened_at.write() = Some(now);
            state.half_open_pass_count.store(0, Ordering::Relaxed);
            state.half_open_in_flight.store(false, Ordering::Relaxed);
            event_sink.emit(ferron_observability::Event::Log(
                ferron_observability::LogEvent {
                    level: ferron_observability::LogLevel::Warn,
                    message: format!(
                        "Upstream {} circuit opened after {} failures within {:?}",
                        upstream.proxy_to, circuit_breaker.max_fails, circuit_breaker.window
                    ),
                    summary: "Upstream circuit opened".into(),
                    target: crate::LOG_TARGET,
                    attributes: vec![(
                        "upstream.address",
                        ferron_observability::LogAttributeValue::String(upstream.proxy_to.clone()),
                    )],
                    trace_context: event_trace_context.clone(),
                },
            ));
            emit_circuit_metric(
                event_sink,
                upstream,
                "ferron.proxy.circuit.state",
                ferron_observability::MetricType::Gauge,
                ferron_observability::MetricValue::U64(2), // Open = 2
                event_trace_context.clone(),
            );
            emit_circuit_metric(
                event_sink,
                upstream,
                "ferron.proxy.circuit.open_total",
                ferron_observability::MetricType::Counter,
                ferron_observability::MetricValue::I64(1),
                event_trace_context,
            );
            true
        }
        _ => false,
    }
}

fn record_circuit_breaker_success(
    circuit_breaker_state: Option<&CircuitBreakerStateMap>,
    circuit_breaker: &CircuitBreakerConfig,
    upstream: &Arc<UpstreamInner>,
    event_sink: &ferron_observability::CompositeEventSink,
    event_trace_context: Option<ferron_observability::EventTraceContext>,
) {
    if !circuit_breaker.enabled {
        return;
    }

    let Some(circuit_breaker_state) = circuit_breaker_state else {
        return;
    };

    // Use `get` instead of `get_mut` for fast path.
    if circuit_breaker_state
        .get(upstream)
        .is_none_or(|s| s.status.load(Ordering::Relaxed) != CIRCUIT_BREAKER_STATUS_HALFOPEN)
    {
        return;
    }

    let Some(state) = circuit_breaker_state.get(upstream) else {
        return;
    };

    state.half_open_in_flight.store(false, Ordering::Relaxed);
    let half_open_pass_count = state.half_open_pass_count.fetch_add(1, Ordering::Relaxed) + 1;

    if half_open_pass_count >= circuit_breaker.consecutive_passes {
        state
            .status
            .store(CIRCUIT_BREAKER_STATUS_CLOSED, Ordering::Relaxed);
        *state.opened_at.write() = None;
        state.half_open_pass_count.store(0, Ordering::Relaxed);
        if let Some(rf) = &state.recent_failures {
            while rf.pop().is_some() {}
        }
        event_sink.emit(ferron_observability::Event::Log(
            ferron_observability::LogEvent {
                level: ferron_observability::LogLevel::Info,
                message: format!(
                    "Upstream {} circuit closed after {} successful half-open request(s)",
                    upstream.proxy_to, circuit_breaker.consecutive_passes
                ),
                summary: "Upstream circuit closed".into(),
                target: crate::LOG_TARGET,
                attributes: vec![(
                    "upstream.address",
                    ferron_observability::LogAttributeValue::String(upstream.proxy_to.clone()),
                )],
                trace_context: event_trace_context.clone(),
            },
        ));
        emit_circuit_metric(
            event_sink,
            upstream,
            "ferron.proxy.circuit.state",
            ferron_observability::MetricType::Gauge,
            ferron_observability::MetricValue::U64(0), // Closed = 0
            event_trace_context.clone(),
        );
        emit_circuit_metric(
            event_sink,
            upstream,
            "ferron.proxy.circuit.open_total",
            ferron_observability::MetricType::Counter,
            ferron_observability::MetricValue::I64(-1),
            event_trace_context,
        );
    }
}

fn is_circuit_breaker_failure_status(status: u16) -> bool {
    (500..600).contains(&status)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicU8};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use dashmap::DashMap;
    use parking_lot::RwLock;
    use rustc_hash::FxBuildHasher;

    use crate::types::circuit::CircuitBreakerState;
    use crate::types::upstream::UpstreamInner;
    use crate::upstream::determine_proxy_to;
    use crate::upstream::lb::{
        ConsistentHashRing, LoadBalancerAlgorithmInner, WeightedRoundRobinState,
    };

    use super::*;

    fn make_upstream(url: &str) -> Arc<UpstreamInner> {
        Arc::new(UpstreamInner {
            proxy_to: url.to_string(),
            proxy_unix: None,
            weight: 1,
            mtls: None,
        })
    }

    #[test]
    fn test_circuit_breaker_opens_after_transport_failures() {
        let circuit_breaker_state: CircuitBreakerStateMap =
            Arc::new(DashMap::with_hasher(FxBuildHasher));
        let upstream = make_upstream("http://backend1");
        let mut metrics = crate::ProxyMetrics::new();
        let circuit_breaker = crate::config::CircuitBreakerConfig {
            enabled: true,
            max_fails: 2,
            window: Duration::from_secs(30),
            open_duration: Duration::from_secs(30),
            consecutive_passes: 1,
            record_5xx: false,
        };

        record_backend_transport_failure(
            Some(&circuit_breaker_state),
            &circuit_breaker,
            &upstream,
            &mut metrics,
            &ferron_observability::CompositeEventSink::new(vec![]),
            None,
        );
        assert!(is_circuit_breaker_available(
            Some(&circuit_breaker_state),
            &circuit_breaker,
            &upstream,
        ));

        record_backend_transport_failure(
            Some(&circuit_breaker_state),
            &circuit_breaker,
            &upstream,
            &mut metrics,
            &ferron_observability::CompositeEventSink::new(vec![]),
            None,
        );

        assert!(!is_circuit_breaker_available(
            Some(&circuit_breaker_state),
            &circuit_breaker,
            &upstream,
        ));
        assert_eq!(metrics.circuit_breaker_unhealthy_backends, vec![upstream]);
    }

    #[test]
    fn test_determine_proxy_to_skips_open_circuit_breaker_backend() {
        let upstreams = vec![
            make_upstream("http://backend1"),
            make_upstream("http://backend2"),
        ];
        let circuit_breaker_state: CircuitBreakerStateMap =
            Arc::new(DashMap::with_hasher(FxBuildHasher));
        let circuit_breaker = crate::config::CircuitBreakerConfig {
            enabled: true,
            max_fails: 1,
            window: Duration::from_secs(30),
            open_duration: Duration::from_secs(30),
            consecutive_passes: 1,
            record_5xx: false,
        };

        circuit_breaker_state.insert(
            upstreams[0].clone(),
            CircuitBreakerState {
                status: Arc::new(AtomicU8::new(CIRCUIT_BREAKER_STATUS_OPEN)),
                opened_at: Arc::new(RwLock::new(Some(Instant::now()))),
                ..Default::default()
            },
        );

        let result = determine_proxy_to(
            &upstreams,
            &LoadBalancerAlgorithmInner::RoundRobin(WeightedRoundRobinState::new()),
            None,
            None,
            None,
            &circuit_breaker,
            Some(&circuit_breaker_state),
            None,
            None,
            &RwLock::new(ConsistentHashRing::new(&[])),
            &ferron_observability::CompositeEventSink::new(vec![]),
            &mut crate::ProxyMetrics::new(),
            None,
        )
        .unwrap();

        assert_eq!(result.upstream.proxy_to, "http://backend2");
    }

    #[test]
    fn test_circuit_breaker_transitions_to_half_open_and_closes_after_success() {
        let circuit_breaker_state: CircuitBreakerStateMap =
            Arc::new(DashMap::with_hasher(FxBuildHasher));
        let upstream = make_upstream("http://backend1");
        let circuit_breaker = crate::config::CircuitBreakerConfig {
            enabled: true,
            max_fails: 1,
            window: Duration::from_secs(30),
            open_duration: Duration::from_secs(1),
            consecutive_passes: 1,
            record_5xx: false,
        };

        circuit_breaker_state.insert(
            upstream.clone(),
            CircuitBreakerState {
                status: Arc::new(AtomicU8::new(CIRCUIT_BREAKER_STATUS_OPEN)),
                opened_at: Arc::new(RwLock::new(Some(Instant::now() - Duration::from_secs(2)))),
                ..Default::default()
            },
        );

        assert!(try_acquire_circuit_breaker_slot(
            Some(&circuit_breaker_state),
            &circuit_breaker,
            &upstream,
            &ferron_observability::CompositeEventSink::new(vec![]),
            None
        ));
        assert!(!is_circuit_breaker_available(
            Some(&circuit_breaker_state),
            &circuit_breaker,
            &upstream,
        ));

        record_backend_response(
            Some(&circuit_breaker_state),
            &circuit_breaker,
            &upstream,
            200,
            &mut crate::ProxyMetrics::new(),
            &ferron_observability::CompositeEventSink::new(vec![]),
            None,
        );

        assert!(is_circuit_breaker_available(
            Some(&circuit_breaker_state),
            &circuit_breaker,
            &upstream,
        ));
    }

    #[test]
    fn test_circuit_breaker_reopens_after_half_open_failure() {
        let circuit_breaker_state: CircuitBreakerStateMap =
            Arc::new(DashMap::with_hasher(FxBuildHasher));
        let upstream = make_upstream("http://backend1");
        let circuit_breaker = crate::config::CircuitBreakerConfig {
            enabled: true,
            max_fails: 1,
            window: Duration::from_secs(30),
            open_duration: Duration::from_secs(30),
            consecutive_passes: 1,
            record_5xx: false,
        };

        circuit_breaker_state.insert(
            upstream.clone(),
            CircuitBreakerState {
                status: Arc::new(AtomicU8::new(CIRCUIT_BREAKER_STATUS_HALFOPEN)),
                half_open_in_flight: Arc::new(AtomicBool::new(true)),
                ..Default::default()
            },
        );

        let mut metrics = crate::ProxyMetrics::new();
        record_backend_transport_failure(
            Some(&circuit_breaker_state),
            &circuit_breaker,
            &upstream,
            &mut metrics,
            &ferron_observability::CompositeEventSink::new(vec![]),
            None,
        );

        assert!(!is_circuit_breaker_available(
            Some(&circuit_breaker_state),
            &circuit_breaker,
            &upstream,
        ));
        assert_eq!(metrics.circuit_breaker_unhealthy_backends, vec![upstream]);
    }

    #[test]
    fn test_circuit_breaker_ignores_5xx_when_record_5xx_false() {
        let circuit_breaker_state: CircuitBreakerStateMap =
            Arc::new(DashMap::with_hasher(FxBuildHasher));
        let upstream = make_upstream("http://backend1");
        let circuit_breaker = crate::config::CircuitBreakerConfig {
            enabled: true,
            max_fails: 1,
            window: Duration::from_secs(30),
            open_duration: Duration::from_secs(30),
            consecutive_passes: 1,
            record_5xx: false,
        };

        // A 500 response should NOT trip the circuit when record_5xx is false
        record_backend_response(
            Some(&circuit_breaker_state),
            &circuit_breaker,
            &upstream,
            500,
            &mut crate::ProxyMetrics::new(),
            &ferron_observability::CompositeEventSink::new(vec![]),
            None,
        );

        assert!(is_circuit_breaker_available(
            Some(&circuit_breaker_state),
            &circuit_breaker,
            &upstream,
        ));
    }

    #[test]
    fn test_circuit_breaker_trips_on_5xx_when_record_5xx_true() {
        let circuit_breaker_state: CircuitBreakerStateMap =
            Arc::new(DashMap::with_hasher(FxBuildHasher));
        let upstream = make_upstream("http://backend1");
        let circuit_breaker = crate::config::CircuitBreakerConfig {
            enabled: true,
            max_fails: 1,
            window: Duration::from_secs(30),
            open_duration: Duration::from_secs(30),
            consecutive_passes: 1,
            record_5xx: true,
        };

        // A 500 response SHOULD trip the circuit when record_5xx is true
        let mut metrics = crate::ProxyMetrics::new();
        record_backend_response(
            Some(&circuit_breaker_state),
            &circuit_breaker,
            &upstream,
            500,
            &mut metrics,
            &ferron_observability::CompositeEventSink::new(vec![]),
            None,
        );

        assert!(!is_circuit_breaker_available(
            Some(&circuit_breaker_state),
            &circuit_breaker,
            &upstream,
        ));
        assert_eq!(metrics.circuit_breaker_unhealthy_backends, vec![upstream]);
    }
}
