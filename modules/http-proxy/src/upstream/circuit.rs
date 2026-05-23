//! Circuit breaker implementation for protecting upstream backends.

use std::sync::Arc;

use crate::config::CircuitBreakerConfig;
use crate::types::circuit::{CircuitBreakerState, CircuitBreakerStateMap, CircuitBreakerStatus};
use crate::types::upstream::UpstreamInner;
use crate::util::FailureCache;

/// Returns whether a backend is currently available for new circuit-breaker traffic.
pub fn is_circuit_breaker_available(
    circuit_breaker_state: Option<&CircuitBreakerStateMap>,
    circuit_breaker: &CircuitBreakerConfig,
    upstream: &UpstreamInner,
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

    match state.status {
        CircuitBreakerStatus::Closed => true,
        CircuitBreakerStatus::Open => state
            .opened_at
            .is_some_and(|opened_at| opened_at.elapsed() >= circuit_breaker.open_duration),
        CircuitBreakerStatus::HalfOpen => !state.half_open_in_flight,
    }
}

/// Record a transport-level backend failure.
pub fn record_backend_transport_failure(
    failed_backends: Arc<FailureCache>,
    passive_check_enabled: bool,
    circuit_breaker_state: Option<&CircuitBreakerStateMap>,
    circuit_breaker: &CircuitBreakerConfig,
    upstream: &UpstreamInner,
    metrics: &mut crate::ProxyMetrics,
    event_sink: &ferron_observability::CompositeEventSink,
) {
    if passive_check_enabled {
        metrics.unhealthy_backends.push(upstream.clone());
        let mut failed = failed_backends.write();
        let current = failed.get(upstream).unwrap_or(0);
        failed.insert(upstream.clone(), current + 1);
    }

    if record_circuit_breaker_failure(circuit_breaker_state, circuit_breaker, upstream, event_sink)
    {
        metrics
            .circuit_breaker_unhealthy_backends
            .push(upstream.clone());
    }
}

/// Record an upstream response for the circuit breaker state machine.
pub fn record_backend_response(
    circuit_breaker_state: Option<&CircuitBreakerStateMap>,
    circuit_breaker: &CircuitBreakerConfig,
    upstream: &UpstreamInner,
    status: u16,
    metrics: &mut crate::ProxyMetrics,
    event_sink: &ferron_observability::CompositeEventSink,
) {
    let should_open = if is_circuit_breaker_failure_status(status) {
        record_circuit_breaker_failure(circuit_breaker_state, circuit_breaker, upstream, event_sink)
    } else {
        record_circuit_breaker_success(
            circuit_breaker_state,
            circuit_breaker,
            upstream,
            event_sink,
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
    upstream: &UpstreamInner,
    event_sink: &ferron_observability::CompositeEventSink,
) -> bool {
    if !circuit_breaker.enabled {
        return true;
    }

    let Some(circuit_breaker_state) = circuit_breaker_state else {
        return true;
    };

    let mut state = circuit_breaker_state.entry(upstream.clone()).or_default();

    match state.status {
        CircuitBreakerStatus::Closed => true,
        CircuitBreakerStatus::Open => {
            let Some(opened_at) = state.opened_at else {
                return false;
            };

            if opened_at.elapsed() < circuit_breaker.open_duration {
                return false;
            }

            state.status = CircuitBreakerStatus::HalfOpen;
            state.opened_at = None;
            state.half_open_in_flight = true;
            state.half_open_pass_count = 0;
            event_sink.emit(ferron_observability::Event::Log(
                ferron_observability::LogEvent {
                    level: ferron_observability::LogLevel::Info,
                    message: format!(
                        "Upstream {} circuit transitioned to half-open",
                        upstream.proxy_to
                    ),
                    target: crate::LOG_TARGET,
                    trace_context: None,
                },
            ));
            true
        }
        CircuitBreakerStatus::HalfOpen => {
            if state.half_open_in_flight {
                false
            } else {
                state.half_open_in_flight = true;
                true
            }
        }
    }
}

fn record_circuit_breaker_failure(
    circuit_breaker_state: Option<&CircuitBreakerStateMap>,
    circuit_breaker: &CircuitBreakerConfig,
    upstream: &UpstreamInner,
    event_sink: &ferron_observability::CompositeEventSink,
) -> bool {
    if !circuit_breaker.enabled {
        return false;
    }

    let Some(circuit_breaker_state) = circuit_breaker_state else {
        return false;
    };

    let now = std::time::Instant::now();
    let mut state = circuit_breaker_state.entry(upstream.clone()).or_default();

    match state.status {
        CircuitBreakerStatus::HalfOpen => {
            state.half_open_in_flight = false;
            state.half_open_pass_count = 0;
            state.recent_failures.clear();
            state.status = CircuitBreakerStatus::Open;
            state.opened_at = Some(now);
            event_sink.emit(ferron_observability::Event::Log(
                ferron_observability::LogEvent {
                    level: ferron_observability::LogLevel::Warn,
                    message: format!(
                        "Upstream {} circuit reopened after a half-open trial failure",
                        upstream.proxy_to
                    ),
                    target: crate::LOG_TARGET,
                    trace_context: None,
                },
            ));
            true
        }
        CircuitBreakerStatus::Open => {
            state.opened_at = Some(now);
            false
        }
        CircuitBreakerStatus::Closed => {
            prune_circuit_breaker_failures(&mut state, circuit_breaker.window, now);
            state.recent_failures.push_back(now);

            if state.recent_failures.len() as u64 >= circuit_breaker.max_fails {
                state.recent_failures.clear();
                state.status = CircuitBreakerStatus::Open;
                state.opened_at = Some(now);
                state.half_open_pass_count = 0;
                state.half_open_in_flight = false;
                ferron_core::log_warn!(
                    "Upstream {} circuit opened after {} failures within {:?}",
                    upstream.proxy_to,
                    circuit_breaker.max_fails,
                    circuit_breaker.window
                );
                true
            } else {
                false
            }
        }
    }
}

fn record_circuit_breaker_success(
    circuit_breaker_state: Option<&CircuitBreakerStateMap>,
    circuit_breaker: &CircuitBreakerConfig,
    upstream: &UpstreamInner,
    event_sink: &ferron_observability::CompositeEventSink,
) {
    if !circuit_breaker.enabled {
        return;
    }

    let Some(circuit_breaker_state) = circuit_breaker_state else {
        return;
    };

    let Some(mut state) = circuit_breaker_state.get_mut(upstream) else {
        return;
    };

    if state.status != CircuitBreakerStatus::HalfOpen {
        return;
    }

    state.half_open_in_flight = false;
    state.half_open_pass_count += 1;

    if state.half_open_pass_count >= circuit_breaker.consecutive_passes {
        state.status = CircuitBreakerStatus::Closed;
        state.opened_at = None;
        state.half_open_pass_count = 0;
        state.recent_failures.clear();
        event_sink.emit(ferron_observability::Event::Log(
            ferron_observability::LogEvent {
                level: ferron_observability::LogLevel::Info,
                message: format!(
                    "Upstream {} circuit closed after {} successful half-open request(s)",
                    upstream.proxy_to, circuit_breaker.consecutive_passes
                ),
                target: crate::LOG_TARGET,
                trace_context: None,
            },
        ));
    }
}

fn prune_circuit_breaker_failures(
    state: &mut CircuitBreakerState,
    window: std::time::Duration,
    now: std::time::Instant,
) {
    while state
        .recent_failures
        .front()
        .is_some_and(|timestamp| now.duration_since(*timestamp) >= window)
    {
        state.recent_failures.pop_front();
    }
}

fn is_circuit_breaker_failure_status(status: u16) -> bool {
    (500..600).contains(&status)
}
