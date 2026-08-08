#![cfg(test)]

use std::sync::atomic::{AtomicBool, AtomicU8};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use parking_lot::RwLock;
use rustc_hash::FxBuildHasher;

use crate::types::circuit::CircuitBreakerState;
use crate::types::upstream::UpstreamInner;
use crate::upstream::lb::{
    ConsistentHashRing, LoadBalancerAlgorithmInner, WeightedRoundRobinState,
};
use crate::upstream::BackendSet;

use super::*;

#[inline]
fn make_upstream(url: &str) -> Arc<UpstreamInner> {
    Arc::new(UpstreamInner {
        proxy_to: url.to_string(),
        connect_to: None,
        proxy_unix: None,
        weight: 1,
        mtls: None,
        priority: 0,
        connection_timeout: None,
        idle_timeout: std::time::Duration::from_secs(60),
        limit: None,
        dns_status: Default::default(),
    })
}

#[inline]
fn cb_view<'a>(
    state: &'a CircuitBreakerStateMap,
    config: &'a crate::config::CircuitBreakerConfig,
    sink: &'a ferron_observability::CompositeEventSink,
) -> CircuitBreaker<'a> {
    CircuitBreaker::new(Some(state), None, config, sink, None, false)
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
        latency_threshold: None,
        flapping_transitions: 3,
        flapping_window: Duration::from_secs(10),
        slow_start_duration: Duration::ZERO,
    };

    record_backend_transport_failure(
        Some(&circuit_breaker_state),
        None,
        &circuit_breaker,
        &upstream,
        &mut metrics,
        &ferron_observability::CompositeEventSink::new(vec![]),
        None,
        false,
    );
    let sink = ferron_observability::CompositeEventSink::new(vec![]);
    let cb = cb_view(&circuit_breaker_state, &circuit_breaker, &sink);
    assert!(cb.is_available(&upstream));

    record_backend_transport_failure(
        Some(&circuit_breaker_state),
        None,
        &circuit_breaker,
        &upstream,
        &mut metrics,
        &ferron_observability::CompositeEventSink::new(vec![]),
        None,
        false,
    );

    let sink = ferron_observability::CompositeEventSink::new(vec![]);
    let cb = cb_view(&circuit_breaker_state, &circuit_breaker, &sink);
    assert!(!cb.is_available(&upstream));
    assert_eq!(metrics.circuit_breaker_unhealthy_backends, vec![upstream]);
}

#[test]
fn test_backend_set_skips_open_circuit_breaker_backend() {
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
        latency_threshold: None,
        flapping_transitions: 3,
        flapping_window: Duration::from_secs(10),
        slow_start_duration: Duration::ZERO,
    };

    circuit_breaker_state.insert(
        upstreams[0].clone(),
        CircuitBreakerState {
            status: Arc::new(AtomicU8::new(CIRCUIT_BREAKER_STATUS_OPEN)),
            opened_at: Arc::new(RwLock::new(Some(Instant::now()))),
            ..Default::default()
        },
    );

    let sink = ferron_observability::CompositeEventSink::new(vec![]);
    let cb = cb_view(&circuit_breaker_state, &circuit_breaker, &sink);
    let ring = RwLock::new(ConsistentHashRing::new(&[]));
    let algorithm = LoadBalancerAlgorithmInner::RoundRobin(WeightedRoundRobinState::new());
    let mut backend_set = BackendSet::new(
        &upstreams, &algorithm, None, None, None, cb, None, None, &ring,
    );
    let result = backend_set.next_backend().unwrap();

    assert_eq!(result.upstream.proxy_to, "http://backend2");
}

#[test]
fn test_try_acquire_delegates_decision_to_is_available() {
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
        latency_threshold: None,
        flapping_transitions: 3,
        flapping_window: Duration::from_secs(10),
        slow_start_duration: Duration::ZERO,
    };
    let sink = ferron_observability::CompositeEventSink::new(vec![]);
    let cb = cb_view(&circuit_breaker_state, &circuit_breaker, &sink);

    // Half-open with the probe slot busy: unavailable and rejected without transition.
    circuit_breaker_state.insert(
        upstream.clone(),
        CircuitBreakerState {
            status: Arc::new(AtomicU8::new(CIRCUIT_BREAKER_STATUS_HALFOPEN)),
            half_open_in_flight: Arc::new(AtomicBool::new(true)),
            ..Default::default()
        },
    );
    assert!(!cb.is_available(&upstream));
    assert!(!cb.try_acquire(&upstream));
    let state = circuit_breaker_state.get(&upstream).unwrap();
    assert_eq!(
        state.status.load(Ordering::Relaxed),
        CIRCUIT_BREAKER_STATUS_HALFOPEN
    );
    drop(state);

    // Open within the cooldown window: unavailable and rejected without transition.
    circuit_breaker_state.insert(
        upstream.clone(),
        CircuitBreakerState {
            status: Arc::new(AtomicU8::new(CIRCUIT_BREAKER_STATUS_OPEN)),
            opened_at: Arc::new(RwLock::new(Some(Instant::now()))),
            ..Default::default()
        },
    );
    assert!(!cb.is_available(&upstream));
    assert!(!cb.try_acquire(&upstream));
    circuit_breaker_state.insert(
        upstream.clone(),
        CircuitBreakerState {
            status: Arc::new(AtomicU8::new(CIRCUIT_BREAKER_STATUS_OPEN)),
            opened_at: Arc::new(RwLock::new(Some(Instant::now() - Duration::from_secs(2)))),
            ..Default::default()
        },
    );
    assert!(cb.is_available(&upstream));
    assert!(cb.try_acquire(&upstream));
    let state = circuit_breaker_state.get(&upstream).unwrap();
    assert_eq!(
        state.status.load(Ordering::Relaxed),
        CIRCUIT_BREAKER_STATUS_HALFOPEN
    );
    assert!(state.half_open_in_flight.load(Ordering::Relaxed));
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
        latency_threshold: None,
        flapping_transitions: 3,
        flapping_window: Duration::from_secs(10),
        slow_start_duration: Duration::ZERO,
    };

    circuit_breaker_state.insert(
        upstream.clone(),
        CircuitBreakerState {
            status: Arc::new(AtomicU8::new(CIRCUIT_BREAKER_STATUS_OPEN)),
            opened_at: Arc::new(RwLock::new(Some(Instant::now() - Duration::from_secs(2)))),
            ..Default::default()
        },
    );

    let sink = ferron_observability::CompositeEventSink::new(vec![]);
    let cb = cb_view(&circuit_breaker_state, &circuit_breaker, &sink);
    assert!(cb.try_acquire(&upstream));
    let sink = ferron_observability::CompositeEventSink::new(vec![]);
    let cb = cb_view(&circuit_breaker_state, &circuit_breaker, &sink);
    assert!(!cb.is_available(&upstream));

    record_backend_response(
        Some(&circuit_breaker_state),
        None,
        &circuit_breaker,
        &upstream,
        200,
        None,
        &mut crate::ProxyMetrics::new(),
        &ferron_observability::CompositeEventSink::new(vec![]),
        None,
        false,
    );

    let sink = ferron_observability::CompositeEventSink::new(vec![]);
    let cb = cb_view(&circuit_breaker_state, &circuit_breaker, &sink);
    assert!(cb.is_available(&upstream));
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
        latency_threshold: None,
        flapping_transitions: 3,
        flapping_window: Duration::from_secs(10),
        slow_start_duration: Duration::ZERO,
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
        None,
        &circuit_breaker,
        &upstream,
        &mut metrics,
        &ferron_observability::CompositeEventSink::new(vec![]),
        None,
        false,
    );

    let sink = ferron_observability::CompositeEventSink::new(vec![]);
    let cb = cb_view(&circuit_breaker_state, &circuit_breaker, &sink);
    assert!(!cb.is_available(&upstream));
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
        latency_threshold: None,
        flapping_transitions: 3,
        flapping_window: Duration::from_secs(10),
        slow_start_duration: Duration::ZERO,
    };

    // A 500 response should NOT trip the circuit when record_5xx is false
    record_backend_response(
        Some(&circuit_breaker_state),
        None,
        &circuit_breaker,
        &upstream,
        500,
        None,
        &mut crate::ProxyMetrics::new(),
        &ferron_observability::CompositeEventSink::new(vec![]),
        None,
        false,
    );

    let sink = ferron_observability::CompositeEventSink::new(vec![]);
    let cb = cb_view(&circuit_breaker_state, &circuit_breaker, &sink);
    assert!(cb.is_available(&upstream));
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
        latency_threshold: None,
        flapping_transitions: 3,
        flapping_window: Duration::from_secs(10),
        slow_start_duration: Duration::ZERO,
    };

    // A 500 response SHOULD trip the circuit when record_5xx is true
    let mut metrics = crate::ProxyMetrics::new();
    record_backend_response(
        Some(&circuit_breaker_state),
        None,
        &circuit_breaker,
        &upstream,
        500,
        None,
        &mut metrics,
        &ferron_observability::CompositeEventSink::new(vec![]),
        None,
        false,
    );

    let sink = ferron_observability::CompositeEventSink::new(vec![]);
    let cb = cb_view(&circuit_breaker_state, &circuit_breaker, &sink);
    assert!(!cb.is_available(&upstream));
    assert_eq!(metrics.circuit_breaker_unhealthy_backends, vec![upstream]);
}

#[test]
fn test_circuit_breaker_trips_on_high_latency() {
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
        latency_threshold: Some(Duration::from_millis(100)),
        flapping_transitions: 3,
        flapping_window: Duration::from_secs(10),
        slow_start_duration: Duration::ZERO,
    };

    // A 200 response with 200ms latency SHOULD trip the circuit when latency_threshold is 100ms
    let mut metrics = crate::ProxyMetrics::new();
    record_backend_response(
        Some(&circuit_breaker_state),
        None,
        &circuit_breaker,
        &upstream,
        200,
        Some(0.2),
        &mut metrics,
        &ferron_observability::CompositeEventSink::new(vec![]),
        None,
        false,
    );

    let sink = ferron_observability::CompositeEventSink::new(vec![]);
    let cb = cb_view(&circuit_breaker_state, &circuit_breaker, &sink);
    assert!(!cb.is_available(&upstream));
    assert_eq!(metrics.circuit_breaker_unhealthy_backends, vec![upstream]);
}

#[test]
fn test_circuit_breaker_ignores_latency_when_not_configured() {
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
        latency_threshold: None,
        flapping_transitions: 3,
        flapping_window: Duration::from_secs(10),
        slow_start_duration: Duration::ZERO,
    };

    // A 200 response with high latency should NOT trip the circuit when latency_threshold is None
    record_backend_response(
        Some(&circuit_breaker_state),
        None,
        &circuit_breaker,
        &upstream,
        200,
        Some(5.0),
        &mut crate::ProxyMetrics::new(),
        &ferron_observability::CompositeEventSink::new(vec![]),
        None,
        false,
    );

    let sink = ferron_observability::CompositeEventSink::new(vec![]);
    let cb = cb_view(&circuit_breaker_state, &circuit_breaker, &sink);
    assert!(cb.is_available(&upstream));
}

#[test]
fn test_circuit_breaker_latency_below_threshold_does_not_trip() {
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
        latency_threshold: Some(Duration::from_millis(100)),
        flapping_transitions: 3,
        flapping_window: Duration::from_secs(10),
        slow_start_duration: Duration::ZERO,
    };

    // A 200 response with 50ms latency should NOT trip the circuit when latency_threshold is 100ms
    record_backend_response(
        Some(&circuit_breaker_state),
        None,
        &circuit_breaker,
        &upstream,
        200,
        Some(0.05),
        &mut crate::ProxyMetrics::new(),
        &ferron_observability::CompositeEventSink::new(vec![]),
        None,
        false,
    );

    let sink = ferron_observability::CompositeEventSink::new(vec![]);
    let cb = cb_view(&circuit_breaker_state, &circuit_breaker, &sink);
    assert!(cb.is_available(&upstream));
}
