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

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use dashmap::DashMap;
    use parking_lot::RwLock;

    use crate::{
        upstream::{
            determine_proxy_to,
            lb::{ConsistentHashRing, LoadBalancerAlgorithmInner, WeightedRoundRobinState},
        },
        util::TtlCache,
    };

    use super::*;

    fn make_upstream(url: &str) -> UpstreamInner {
        UpstreamInner {
            proxy_to: url.to_string(),
            proxy_unix: None,
            weight: 1,
        }
    }

    #[test]
    fn test_circuit_breaker_opens_after_transport_failures() {
        let failed_backends: Arc<RwLock<TtlCache<UpstreamInner, u64>>> =
            Arc::new(RwLock::new(TtlCache::new(Duration::from_secs(60))));
        let circuit_breaker_state: CircuitBreakerStateMap = Arc::new(DashMap::new());
        let upstream = make_upstream("http://backend1");
        let mut metrics = crate::ProxyMetrics::new();
        let circuit_breaker = crate::config::CircuitBreakerConfig {
            enabled: true,
            max_fails: 2,
            window: Duration::from_secs(30),
            open_duration: Duration::from_secs(30),
            consecutive_passes: 1,
        };

        record_backend_transport_failure(
            Arc::clone(&failed_backends),
            false,
            Some(&circuit_breaker_state),
            &circuit_breaker,
            &upstream,
            &mut metrics,
            &ferron_observability::CompositeEventSink::new(vec![]),
        );
        assert!(is_circuit_breaker_available(
            Some(&circuit_breaker_state),
            &circuit_breaker,
            &upstream,
        ));

        record_backend_transport_failure(
            Arc::clone(&failed_backends),
            false,
            Some(&circuit_breaker_state),
            &circuit_breaker,
            &upstream,
            &mut metrics,
            &ferron_observability::CompositeEventSink::new(vec![]),
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
        let failed_backends: Arc<RwLock<TtlCache<UpstreamInner, u64>>> =
            Arc::new(RwLock::new(TtlCache::new(Duration::from_secs(60))));
        let circuit_breaker_state: CircuitBreakerStateMap = Arc::new(DashMap::new());
        let circuit_breaker = crate::config::CircuitBreakerConfig {
            enabled: true,
            max_fails: 1,
            window: Duration::from_secs(30),
            open_duration: Duration::from_secs(30),
            consecutive_passes: 1,
        };

        circuit_breaker_state.insert(
            upstreams[0].clone(),
            CircuitBreakerState {
                status: CircuitBreakerStatus::Open,
                opened_at: Some(Instant::now()),
                ..Default::default()
            },
        );

        let result = determine_proxy_to(
            &upstreams,
            &failed_backends,
            false,
            3,
            &LoadBalancerAlgorithmInner::RoundRobin(WeightedRoundRobinState::new()),
            None,
            None,
            &circuit_breaker,
            Some(&circuit_breaker_state),
            &[],
            None,
            None,
            &RwLock::new(ConsistentHashRing::new(&[])),
            &ferron_observability::CompositeEventSink::new(vec![]),
        )
        .unwrap();

        assert_eq!(result.upstream.proxy_to, "http://backend2");
    }

    #[test]
    fn test_circuit_breaker_transitions_to_half_open_and_closes_after_success() {
        let circuit_breaker_state: CircuitBreakerStateMap = Arc::new(DashMap::new());
        let upstream = make_upstream("http://backend1");
        let circuit_breaker = crate::config::CircuitBreakerConfig {
            enabled: true,
            max_fails: 1,
            window: Duration::from_secs(30),
            open_duration: Duration::from_secs(1),
            consecutive_passes: 1,
        };

        circuit_breaker_state.insert(
            upstream.clone(),
            CircuitBreakerState {
                status: CircuitBreakerStatus::Open,
                opened_at: Some(Instant::now() - Duration::from_secs(2)),
                ..Default::default()
            },
        );

        assert!(try_acquire_circuit_breaker_slot(
            Some(&circuit_breaker_state),
            &circuit_breaker,
            &upstream,
            &ferron_observability::CompositeEventSink::new(vec![]),
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
        );

        assert!(is_circuit_breaker_available(
            Some(&circuit_breaker_state),
            &circuit_breaker,
            &upstream,
        ));
    }

    #[test]
    fn test_circuit_breaker_reopens_after_half_open_failure() {
        let circuit_breaker_state: CircuitBreakerStateMap = Arc::new(DashMap::new());
        let upstream = make_upstream("http://backend1");
        let circuit_breaker = crate::config::CircuitBreakerConfig {
            enabled: true,
            max_fails: 1,
            window: Duration::from_secs(30),
            open_duration: Duration::from_secs(30),
            consecutive_passes: 1,
        };

        circuit_breaker_state.insert(
            upstream.clone(),
            CircuitBreakerState {
                status: CircuitBreakerStatus::HalfOpen,
                half_open_in_flight: true,
                ..Default::default()
            },
        );

        let mut metrics = crate::ProxyMetrics::new();
        record_backend_transport_failure(
            Arc::new(RwLock::new(TtlCache::new(Duration::from_secs(60)))),
            false,
            Some(&circuit_breaker_state),
            &circuit_breaker,
            &upstream,
            &mut metrics,
            &ferron_observability::CompositeEventSink::new(vec![]),
        );

        assert!(!is_circuit_breaker_available(
            Some(&circuit_breaker_state),
            &circuit_breaker,
            &upstream,
        ));
        assert_eq!(metrics.circuit_breaker_unhealthy_backends, vec![upstream]);
    }
}
