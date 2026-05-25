use std::{sync::Arc, time::Duration};

use dashmap::DashMap;
use parking_lot::RwLock;

use crate::types::affinity::AffinityType;
use crate::types::health::{HealthCheckState, HealthCheckStateMap};
use crate::types::lb::LoadBalancerAlgorithm;
use crate::types::upstream::UpstreamInner;
use crate::types::ConnectionsTrackState;
use crate::upstream::affinity::resolve_affinity_index;
use crate::upstream::lb::{
    selector::select_backend_index, ConsistentHashRing, EwmaStateMap, LoadBalancerAlgorithmInner,
    WeightedRoundRobinState,
};
use crate::util::TtlCache;

use super::*;

fn make_upstream(url: &str) -> UpstreamInner {
    UpstreamInner {
        proxy_to: url.to_string(),
        proxy_unix: None,
        weight: 1,
    }
}

fn make_upstream_with_weight(url: &str, weight: u32) -> UpstreamInner {
    UpstreamInner {
        proxy_to: url.to_string(),
        proxy_unix: None,
        weight,
    }
}

#[test]
fn test_select_backend_index_random() {
    let backends = vec![
        (0, make_upstream("http://backend1")),
        (1, make_upstream("http://backend2")),
        (2, make_upstream("http://backend3")),
    ];
    let algorithm = LoadBalancerAlgorithmInner::Random;

    for _ in 0..100 {
        let idx = select_backend_index(&algorithm, &backends, None, None);
        assert!(idx < backends.len());
    }
}

#[test]
fn test_select_backend_index_round_robin() {
    let backends = vec![
        (0, make_upstream("http://backend1")),
        (1, make_upstream("http://backend2")),
        (2, make_upstream("http://backend3")),
    ];
    let state = WeightedRoundRobinState::new();
    let algorithm = LoadBalancerAlgorithmInner::RoundRobin(state);

    assert_eq!(select_backend_index(&algorithm, &backends, None, None), 0);
    assert_eq!(select_backend_index(&algorithm, &backends, None, None), 1);
    assert_eq!(select_backend_index(&algorithm, &backends, None, None), 2);
    assert_eq!(select_backend_index(&algorithm, &backends, None, None), 0);
}

#[test]
fn test_select_backend_index_least_connections() {
    let backends = vec![
        (0, make_upstream("http://backend1")),
        (1, make_upstream("http://backend2")),
    ];
    let conn_state: ConnectionsTrackState = Arc::new(DashMap::new());
    let algorithm = LoadBalancerAlgorithmInner::LeastConnections;

    let idx = select_backend_index(&algorithm, &backends, Some(&conn_state), None);
    assert!(idx < backends.len());

    let tracker1 = Arc::new(());
    conn_state.insert(backends[0].1.clone(), tracker1.clone());
    let _clone1 = tracker1.clone();
    let _clone2 = tracker1.clone();

    let idx = select_backend_index(&algorithm, &backends, Some(&conn_state), None);
    assert_eq!(idx, 1);
}

#[test]
fn test_select_backend_index_two_random_choices() {
    let backends = vec![
        (0, make_upstream("http://backend1")),
        (1, make_upstream("http://backend2")),
        (2, make_upstream("http://backend3")),
    ];
    let conn_state: ConnectionsTrackState = Arc::new(DashMap::new());
    let algorithm = LoadBalancerAlgorithmInner::TwoRandomChoices;

    for _ in 0..100 {
        let idx = select_backend_index(&algorithm, &backends, Some(&conn_state), None);
        assert!(idx < backends.len());
    }
}

#[test]
fn test_select_backend_single_backend() {
    let backends = vec![(0, make_upstream("http://backend1"))];
    let conn_state: ConnectionsTrackState = Arc::new(DashMap::new());
    let algorithm = LoadBalancerAlgorithmInner::TwoRandomChoices;

    let idx = select_backend_index(&algorithm, &backends, Some(&conn_state), None);
    assert_eq!(idx, 0);
}

#[test]
fn test_determine_proxy_to_no_upstreams() {
    let failed_backends: Arc<RwLock<TtlCache<UpstreamInner, u64>>> =
        Arc::new(RwLock::new(TtlCache::new(Duration::from_secs(60))));
    let algorithm = LoadBalancerAlgorithmInner::Random;

    let result = determine_proxy_to(
        &[],
        &failed_backends,
        false,
        3,
        &algorithm,
        None,
        None,
        None,
        &crate::config::CircuitBreakerConfig::default(),
        None,
        &[],
        None,
        None,
        &RwLock::new(ConsistentHashRing::new(&[])),
        &ferron_observability::CompositeEventSink::new(vec![]),
    );
    assert!(result.is_none());
}

#[test]
fn test_determine_proxy_to_single_backend() {
    let upstreams = vec![make_upstream("http://backend1")];
    let failed_backends: Arc<RwLock<TtlCache<UpstreamInner, u64>>> =
        Arc::new(RwLock::new(TtlCache::new(Duration::from_secs(60))));
    let algorithm = LoadBalancerAlgorithmInner::Random;
    let conn_state: ConnectionsTrackState = Arc::new(DashMap::new());

    let result = determine_proxy_to(
        &upstreams,
        &failed_backends,
        false,
        3,
        &algorithm,
        Some(&conn_state),
        None,
        None,
        &crate::config::CircuitBreakerConfig::default(),
        None,
        &[],
        None,
        None,
        &RwLock::new(ConsistentHashRing::new(&[])),
        &ferron_observability::CompositeEventSink::new(vec![]),
    );
    assert!(result.is_some());
    let selected = result.unwrap();
    assert_eq!(selected.upstream.proxy_to, "http://backend1");
}

#[test]
fn test_determine_proxy_to_health_check_filters_unhealthy() {
    let upstreams = vec![
        make_upstream("http://backend1"),
        make_upstream("http://backend2"),
    ];
    let failed_backends: Arc<RwLock<TtlCache<UpstreamInner, u64>>> =
        Arc::new(RwLock::new(TtlCache::new(Duration::from_secs(60))));

    {
        let mut failed = failed_backends.write();
        failed.insert(make_upstream("http://backend1"), 5);
    }

    let algorithm = LoadBalancerAlgorithmInner::Random;

    let result = determine_proxy_to(
        &upstreams,
        &failed_backends,
        true,
        3,
        &algorithm,
        None,
        None,
        None,
        &crate::config::CircuitBreakerConfig::default(),
        None,
        &[],
        None,
        None,
        &RwLock::new(ConsistentHashRing::new(&[])),
        &ferron_observability::CompositeEventSink::new(vec![]),
    );
    assert!(result.is_some());
    assert_eq!(result.unwrap().upstream.proxy_to, "http://backend2");
}

#[test]
fn test_determine_proxy_to_all_unhealthy() {
    let upstreams = vec![
        make_upstream("http://backend1"),
        make_upstream("http://backend2"),
    ];
    let failed_backends: Arc<RwLock<TtlCache<UpstreamInner, u64>>> =
        Arc::new(RwLock::new(TtlCache::new(Duration::from_secs(60))));

    {
        let mut failed = failed_backends.write();
        failed.insert(make_upstream("http://backend1"), 5);
        failed.insert(make_upstream("http://backend2"), 5);
    }

    let algorithm = LoadBalancerAlgorithmInner::Random;

    let result = determine_proxy_to(
        &upstreams,
        &failed_backends,
        true,
        3,
        &algorithm,
        None,
        None,
        None,
        &crate::config::CircuitBreakerConfig::default(),
        None,
        &[],
        None,
        None,
        &RwLock::new(ConsistentHashRing::new(&[])),
        &ferron_observability::CompositeEventSink::new(vec![]),
    );
    assert!(result.is_none());
}

#[test]
fn test_determine_proxy_to_health_check_disabled() {
    let upstreams = vec![
        make_upstream("http://backend1"),
        make_upstream("http://backend2"),
    ];
    let failed_backends: Arc<RwLock<TtlCache<UpstreamInner, u64>>> =
        Arc::new(RwLock::new(TtlCache::new(Duration::from_secs(60))));

    {
        let mut failed = failed_backends.write();
        failed.insert(make_upstream("http://backend1"), 100);
    }

    let algorithm = LoadBalancerAlgorithmInner::Random;

    let result = determine_proxy_to(
        &upstreams,
        &failed_backends,
        false,
        3,
        &algorithm,
        None,
        None,
        None,
        &crate::config::CircuitBreakerConfig::default(),
        None,
        &[],
        None,
        None,
        &RwLock::new(ConsistentHashRing::new(&[])),
        &ferron_observability::CompositeEventSink::new(vec![]),
    );
    assert!(result.is_some());
}

#[test]
fn test_record_backend_transport_failure() {
    let failed_backends: Arc<RwLock<TtlCache<UpstreamInner, u64>>> =
        Arc::new(RwLock::new(TtlCache::new(Duration::from_secs(60))));
    let upstream = make_upstream("http://backend1");
    let mut metrics = crate::ProxyMetrics::new();

    record_backend_transport_failure(
        Arc::clone(&failed_backends),
        true,
        None,
        &crate::config::CircuitBreakerConfig::default(),
        &upstream,
        &mut metrics,
        &ferron_observability::CompositeEventSink::new(vec![]),
    );

    assert_eq!(metrics.unhealthy_backends.len(), 1);
    assert_eq!(failed_backends.read().get(&upstream), Some(1));

    record_backend_transport_failure(
        Arc::clone(&failed_backends),
        true,
        None,
        &crate::config::CircuitBreakerConfig::default(),
        &upstream,
        &mut metrics,
        &ferron_observability::CompositeEventSink::new(vec![]),
    );

    assert_eq!(failed_backends.read().get(&upstream), Some(2));
}

#[test]
fn test_record_backend_transport_failure_passive_check_disabled() {
    let failed_backends: Arc<RwLock<TtlCache<UpstreamInner, u64>>> =
        Arc::new(RwLock::new(TtlCache::new(Duration::from_secs(60))));
    let upstream = make_upstream("http://backend1");
    let mut metrics = crate::ProxyMetrics::new();

    record_backend_transport_failure(
        Arc::clone(&failed_backends),
        false,
        None,
        &crate::config::CircuitBreakerConfig::default(),
        &upstream,
        &mut metrics,
        &ferron_observability::CompositeEventSink::new(vec![]),
    );

    assert_eq!(metrics.unhealthy_backends.len(), 0);
    assert_eq!(failed_backends.read().get(&upstream), None);
}

#[test]
fn test_upstream_inner_debug() {
    let upstream = make_upstream("http://backend1");
    let debug_str = format!("{:?}", upstream);
    assert!(debug_str.contains("http://backend1"));
}

#[test]
fn test_load_balancer_algorithm_from() {
    assert!(matches!(
        LoadBalancerAlgorithmInner::from(LoadBalancerAlgorithm::Random),
        LoadBalancerAlgorithmInner::Random
    ));
    assert!(matches!(
        LoadBalancerAlgorithmInner::from(LoadBalancerAlgorithm::RoundRobin),
        LoadBalancerAlgorithmInner::RoundRobin(_)
    ));
    assert!(matches!(
        LoadBalancerAlgorithmInner::from(LoadBalancerAlgorithm::LeastConnections),
        LoadBalancerAlgorithmInner::LeastConnections
    ));
    assert!(matches!(
        LoadBalancerAlgorithmInner::from(LoadBalancerAlgorithm::TwoRandomChoices),
        LoadBalancerAlgorithmInner::TwoRandomChoices
    ));
}

#[test]
fn test_determine_proxy_to_active_health_check_filters_unhealthy() {
    let upstreams = vec![
        make_upstream("http://backend1"),
        make_upstream("http://backend2"),
    ];
    let failed_backends: Arc<RwLock<TtlCache<UpstreamInner, u64>>> =
        Arc::new(RwLock::new(TtlCache::new(Duration::from_secs(60))));

    let health_check_state: HealthCheckStateMap = Arc::new(DashMap::new());
    health_check_state.insert(
        "http://backend1".to_string(),
        HealthCheckState {
            is_healthy: false,
            ..Default::default()
        },
    );

    let algorithm = LoadBalancerAlgorithmInner::Random;

    let result = determine_proxy_to(
        &upstreams,
        &failed_backends,
        true,
        3,
        &algorithm,
        None,
        None,
        Some(&health_check_state),
        &crate::config::CircuitBreakerConfig::default(),
        None,
        &[],
        None,
        None,
        &RwLock::new(ConsistentHashRing::new(&[])),
        &ferron_observability::CompositeEventSink::new(vec![]),
    );
    assert!(result.is_some());
    assert_eq!(result.unwrap().upstream.proxy_to, "http://backend2");
}

#[test]
fn test_determine_proxy_to_active_health_check_all_healthy() {
    let upstreams = vec![
        make_upstream("http://backend1"),
        make_upstream("http://backend2"),
    ];
    let failed_backends: Arc<RwLock<TtlCache<UpstreamInner, u64>>> =
        Arc::new(RwLock::new(TtlCache::new(Duration::from_secs(60))));

    let health_check_state: HealthCheckStateMap = Arc::new(DashMap::new());
    let algorithm = LoadBalancerAlgorithmInner::Random;

    let result = determine_proxy_to(
        &upstreams,
        &failed_backends,
        true,
        3,
        &algorithm,
        None,
        None,
        Some(&health_check_state),
        &crate::config::CircuitBreakerConfig::default(),
        None,
        &[],
        None,
        None,
        &RwLock::new(ConsistentHashRing::new(&[])),
        &ferron_observability::CompositeEventSink::new(vec![]),
    );
    assert!(result.is_some());
    let selected = result.unwrap();
    assert!(
        selected.upstream.proxy_to == "http://backend1"
            || selected.upstream.proxy_to == "http://backend2"
    );
}

#[test]
fn test_select_backend_index_weighted_round_robin_equal_weights() {
    let backends = vec![
        (0, make_upstream_with_weight("http://backend1", 1)),
        (1, make_upstream_with_weight("http://backend2", 1)),
        (2, make_upstream_with_weight("http://backend3", 1)),
    ];
    let state = WeightedRoundRobinState::new();
    let algorithm = LoadBalancerAlgorithmInner::RoundRobin(state);

    // With equal weights, should cycle like round-robin
    assert_eq!(select_backend_index(&algorithm, &backends, None, None), 0);
    assert_eq!(select_backend_index(&algorithm, &backends, None, None), 1);
    assert_eq!(select_backend_index(&algorithm, &backends, None, None), 2);
    assert_eq!(select_backend_index(&algorithm, &backends, None, None), 0);
}

#[test]
fn test_select_backend_index_weighted_round_robin_unequal_weights() {
    let backends = vec![
        (0, make_upstream_with_weight("http://backend1", 5)),
        (1, make_upstream_with_weight("http://backend2", 1)),
        (2, make_upstream_with_weight("http://backend3", 1)),
    ];
    let state = WeightedRoundRobinState::new();
    let algorithm = LoadBalancerAlgorithmInner::RoundRobin(state);

    // Over 7 selections (total weight), backend1 should be selected 5 times,
    // backend2 and backend3 once each
    let mut counts = [0usize; 3];
    for _ in 0..7 {
        let idx = select_backend_index(&algorithm, &backends, None, None);
        counts[idx] += 1;
    }
    assert_eq!(counts[0], 5);
    assert_eq!(counts[1], 1);
    assert_eq!(counts[2], 1);
}

#[test]
fn test_select_backend_index_weighted_round_robin_smooth_distribution() {
    let backends = vec![
        (0, make_upstream_with_weight("http://backend1", 5)),
        (1, make_upstream_with_weight("http://backend2", 1)),
    ];
    let state = WeightedRoundRobinState::new();
    let algorithm = LoadBalancerAlgorithmInner::RoundRobin(state);

    // With weights 5:1, smooth WRR should distribute as:
    // A, A, B, A, A, A (not AAAAAA B)
    let selections: Vec<usize> = (0..6)
        .map(|_| select_backend_index(&algorithm, &backends, None, None))
        .collect();

    // Backend1 (weight 5) should be selected 5 times
    let b1_count = selections.iter().filter(|&&x| x == 0).count();
    let b2_count = selections.iter().filter(|&&x| x == 1).count();
    assert_eq!(b1_count, 5);
    assert_eq!(b2_count, 1);

    // Verify smooth distribution: backend2 should not be at the very end
    // In smooth WRR with 5:1, the pattern is typically: 0, 0, 1, 0, 0, 0
    assert!(selections.iter().position(|&x| x == 1).unwrap() < 5);
}

#[test]
fn test_select_backend_index_weighted_round_robin_single_backend() {
    let backends = vec![(0, make_upstream_with_weight("http://backend1", 10))];
    let state = WeightedRoundRobinState::new();
    let algorithm = LoadBalancerAlgorithmInner::RoundRobin(state);

    for _ in 0..10 {
        assert_eq!(select_backend_index(&algorithm, &backends, None, None), 0);
    }
}

#[test]
fn test_weighted_round_robin_state_resize() {
    let state = WeightedRoundRobinState::new();

    // Start with 2 backends
    let weights1 = [3u32, 1];
    let idx1 = state.next(&weights1);
    assert!(idx1 < 2);

    // Resize to 3 backends
    let weights2 = [3u32, 1, 2];
    let idx2 = state.next(&weights2);
    assert!(idx2 < 3);

    // Resize back to 2 backends
    let idx3 = state.next(&weights1);
    assert!(idx3 < 2);
}

#[test]
fn test_resolve_affinity_index_ip_affinity() {
    let backends = vec![
        make_upstream("http://backend1"),
        make_upstream("http://backend2"),
        make_upstream("http://backend3"),
    ];
    let affinity_type = AffinityType::Ip;

    // IP affinity uses consistent hash ring or modulus
    let key = b"192.168.1.1";
    let idx = resolve_affinity_index(
        &affinity_type,
        key,
        &backends,
        &RwLock::new(ConsistentHashRing::new(&backends)),
    );
    assert!(idx.is_some());
    assert!(idx.unwrap() < backends.len());

    // Same IP should always map to the same backend
    let idx2 = resolve_affinity_index(
        &affinity_type,
        key,
        &backends,
        &RwLock::new(ConsistentHashRing::new(&backends)),
    );
    assert_eq!(idx, idx2);
}

// ============================================================================
// Weighted Least Connections Tests
// ============================================================================

#[test]
fn test_select_backend_index_least_connections_fewer_connections() {
    let backends = vec![
        (0, make_upstream("http://backend1")),
        (1, make_upstream("http://backend2")),
    ];
    let conn_state: ConnectionsTrackState = Arc::new(DashMap::new());
    let algorithm = LoadBalancerAlgorithmInner::LeastConnections;

    // Simulate 3 connections on backend1, 0 on backend2
    let tracker1 = Arc::new(());
    let _conn2 = tracker1.clone();
    let _conn3 = tracker1.clone();
    let tracker2 = Arc::new(());
    conn_state.insert(backends[0].1.clone(), tracker1);
    conn_state.insert(backends[1].1.clone(), tracker2);

    // Should pick backend2 (fewer connections)
    let idx = select_backend_index(&algorithm, &backends, Some(&conn_state), None);
    assert_eq!(idx, 1);
}

#[test]
fn test_select_backend_index_least_connections_weighted_higher_weight_favored() {
    let backends = vec![
        (0, make_upstream_with_weight("http://backend1", 1)),
        (1, make_upstream_with_weight("http://backend2", 3)),
    ];
    let conn_state: ConnectionsTrackState = Arc::new(DashMap::new());
    let algorithm = LoadBalancerAlgorithmInner::LeastConnections;

    // Simulate 3 connections on backend1 (weight 1) and 6 connections on backend2 (weight 3)
    // Score for backend1: 3 * 3 = 9 (using backend2's weight)
    // Score for backend2: 6 * 1 = 6 (using backend1's weight)
    // Backend2 has lower score, so it should be selected
    let mut trackers = vec![];
    let tracker1 = Arc::new(());
    let tracker2 = Arc::new(());
    for _ in 1..3 {
        trackers.push(tracker1.clone());
    }
    for _ in 1..6 {
        trackers.push(tracker2.clone());
    }
    conn_state.insert(backends[0].1.clone(), tracker1);
    conn_state.insert(backends[1].1.clone(), tracker2);

    // Should pick backend2 (higher weight compensates for more connections)
    let idx = select_backend_index(&algorithm, &backends, Some(&conn_state), None);
    assert_eq!(idx, 1);
}

#[test]
fn test_select_backend_index_least_connections_weighted_lower_weight_favored() {
    let backends = vec![
        (0, make_upstream_with_weight("http://backend1", 3)),
        (1, make_upstream_with_weight("http://backend2", 1)),
    ];
    let conn_state: ConnectionsTrackState = Arc::new(DashMap::new());
    let algorithm = LoadBalancerAlgorithmInner::LeastConnections;

    // Simulate 6 connections on backend1 (weight 3) and 3 connections on backend2 (weight 1)
    // Score for backend1: 6 * 1 = 6 (using backend2's weight)
    // Score for backend2: 3 * 3 = 9 (using backend1's weight)
    // Backend1 has lower score, so it should be selected
    let tracker1 = Arc::new(());
    let tracker2 = Arc::new(());
    let mut trackers = vec![];
    for _ in 1..6 {
        trackers.push(tracker1.clone());
    }
    for _ in 1..3 {
        trackers.push(tracker2.clone());
    }
    conn_state.insert(backends[0].1.clone(), tracker1);
    conn_state.insert(backends[1].1.clone(), tracker2);

    // Should pick backend1 (higher weight compensates for more connections)
    let idx = select_backend_index(&algorithm, &backends, Some(&conn_state), None);
    assert_eq!(idx, 0);
}

#[test]
fn test_select_backend_index_least_connections_weighted_equal_score() {
    let backends = vec![
        (0, make_upstream_with_weight("http://backend1", 2)),
        (1, make_upstream_with_weight("http://backend2", 1)),
    ];
    let conn_state: ConnectionsTrackState = Arc::new(DashMap::new());
    let algorithm = LoadBalancerAlgorithmInner::LeastConnections;

    // Simulate 2 connections on backend1 (weight 2) and 4 connections on backend2 (weight 1)
    // Score for backend1: 2 * 1 = 2 (using backend2's weight)
    // Score for backend2: 4 * 2 = 8 (using backend1's weight)
    // Backend1 has lower score, so it should be selected
    let mut trackers = vec![];
    let tracker1 = Arc::new(());
    let tracker2 = Arc::new(());
    for _ in 1..2 {
        trackers.push(tracker1.clone());
    }
    for _ in 1..4 {
        trackers.push(tracker2.clone());
    }
    conn_state.insert(backends[0].1.clone(), tracker1);
    conn_state.insert(backends[1].1.clone(), tracker2);

    // Should pick backend1 (lower weighted score)
    let idx = select_backend_index(&algorithm, &backends, Some(&conn_state), None);
    assert_eq!(idx, 0);
}

#[test]
fn test_select_backend_index_least_connections_all_zero_weight() {
    let backends = vec![
        (0, make_upstream_with_weight("http://backend1", 0)),
        (1, make_upstream_with_weight("http://backend2", 0)),
    ];
    let conn_state: ConnectionsTrackState = Arc::new(DashMap::new());
    let algorithm = LoadBalancerAlgorithmInner::LeastConnections;

    // All backends have weight 0, should fall back to index 0
    let idx = select_backend_index(&algorithm, &backends, Some(&conn_state), None);
    assert_eq!(idx, 0);
}

#[test]
fn test_select_backend_index_least_connections_weighted_uneven_distribution() {
    let backends = vec![
        (0, make_upstream_with_weight("http://backend1", 1)),
        (1, make_upstream_with_weight("http://backend2", 2)),
        (2, make_upstream_with_weight("http://backend3", 3)),
    ];
    let conn_state: ConnectionsTrackState = Arc::new(DashMap::new());
    let algorithm = LoadBalancerAlgorithmInner::LeastConnections;

    let mut trackers = vec![];
    let tracker1 = Arc::new(());
    let tracker2 = Arc::new(());
    let tracker3 = Arc::new(());

    // Simulate a heavy load on backend1
    for _ in 1..100 {
        trackers.push(tracker1.clone());
    }
    // Moderate load on backend2
    for _ in 1..20 {
        trackers.push(tracker2.clone());
    }
    // Light load on backend3
    for _ in 1..5 {
        trackers.push(tracker3.clone());
    }

    conn_state.insert(backends[0].1.clone(), tracker1);
    conn_state.insert(backends[1].1.clone(), tracker2);
    conn_state.insert(backends[2].1.clone(), tracker3);

    // Backend3 should be selected (fewest connections and highest weight)
    let idx = select_backend_index(&algorithm, &backends, Some(&conn_state), None);
    assert_eq!(idx, 2);
}

// ---------------------------------------------------------------------------
// P2C+EWMA tests
// ---------------------------------------------------------------------------

#[test]
fn test_select_backend_index_p2c_ewma_basic() {
    let backends = vec![
        (0, make_upstream("http://backend1")),
        (1, make_upstream("http://backend2")),
        (2, make_upstream("http://backend3")),
    ];
    let conn_state: ConnectionsTrackState = Arc::new(DashMap::new());
    let ewma_state: EwmaStateMap = Arc::new(DashMap::new());
    let algorithm = LoadBalancerAlgorithmInner::P2cEwma;

    for _ in 0..100 {
        let idx = select_backend_index(&algorithm, &backends, Some(&conn_state), Some(&ewma_state));
        assert!(idx < backends.len());
    }
}

#[test]
fn test_select_backend_index_p2c_ewma_single_backend() {
    let backends = vec![(0, make_upstream("http://backend1"))];
    let conn_state: ConnectionsTrackState = Arc::new(DashMap::new());
    let ewma_state: EwmaStateMap = Arc::new(DashMap::new());
    let algorithm = LoadBalancerAlgorithmInner::P2cEwma;

    let idx = select_backend_index(&algorithm, &backends, Some(&conn_state), Some(&ewma_state));
    assert_eq!(idx, 0);
}

#[test]
fn test_select_backend_index_p2c_ewma_prefers_lower_latency() {
    let backends = vec![
        (0, make_upstream("http://backend1")),
        (1, make_upstream("http://backend2")),
    ];
    let conn_state: ConnectionsTrackState = Arc::new(DashMap::new());
    let ewma_state: EwmaStateMap = Arc::new(DashMap::new());
    let algorithm = LoadBalancerAlgorithmInner::P2cEwma;

    // Initialise trackers so both backends have 0 active connections
    conn_state.entry(backends[0].1.clone()).or_insert(Arc::new(()));
    conn_state.entry(backends[1].1.clone()).or_insert(Arc::new(()));

    // Backend1 starts with a low latency record, Backend2 with a high one
    let params = super::lb::p2c_ewma::P2cEwmaParams::default();
    crate::upstream::lb::p2c_ewma::update_ewma(&ewma_state, &backends[0].1, 0.01, &params);
    crate::upstream::lb::p2c_ewma::update_ewma(&ewma_state, &backends[1].1, 2.0, &params);

    // With equal connection counts, the lower-latency backend should win
    let mut backend1_wins = 0;
    let mut backend2_wins = 0;
    for _ in 0..200 {
        let idx = select_backend_index(&algorithm, &backends, Some(&conn_state), Some(&ewma_state));
        if idx == 0 {
            backend1_wins += 1;
        } else {
            backend2_wins += 1;
        }
    }
    // Backend1 (lower latency) should win significantly more often
    assert!(
        backend1_wins > backend2_wins,
        "expected backend1 (low latency) to be preferred, got {backend1_wins} vs {backend2_wins}"
    );
}

#[test]
fn test_select_backend_index_p2c_ewma_prefers_fewer_connections() {
    let backends = vec![
        (0, make_upstream("http://backend1")),
        (1, make_upstream("http://backend2")),
    ];
    let conn_state: ConnectionsTrackState = Arc::new(DashMap::new());
    let ewma_state: EwmaStateMap = Arc::new(DashMap::new());
    let algorithm = LoadBalancerAlgorithmInner::P2cEwma;

    // Equal EWMA latencies
    let params = super::lb::p2c_ewma::P2cEwmaParams::default();
    crate::upstream::lb::p2c_ewma::update_ewma(&ewma_state, &backends[0].1, 0.1, &params);
    crate::upstream::lb::p2c_ewma::update_ewma(&ewma_state, &backends[1].1, 0.1, &params);

    // Backend1 has many active connections
    let tracker1 = Arc::new(());
    conn_state.insert(backends[0].1.clone(), tracker1.clone());
    // Simulate 10 active connections via strong_count
    let mut conns = Vec::new();
    for _ in 0..10 {
        conns.push(tracker1.clone());
    }

    // Backend2 has few active connections (just the entry tracker)
    let tracker2 = Arc::new(());
    conn_state.insert(backends[1].1.clone(), tracker2.clone());

    let mut backend1_wins = 0;
    let mut backend2_wins = 0;
    for _ in 0..200 {
        let idx = select_backend_index(&algorithm, &backends, Some(&conn_state), Some(&ewma_state));
        if idx == 0 {
            backend1_wins += 1;
        } else {
            backend2_wins += 1;
        }
    }
    // Backend2 (fewer connections) should win significantly more often
    assert!(
        backend2_wins > backend1_wins,
        "expected backend2 (fewer active connections) to be preferred, got {backend2_wins} vs {backend1_wins}"
    );
}

#[test]
fn test_p2c_ewma_update() {
    let ewma_state: EwmaStateMap = Arc::new(DashMap::new());
    let upstream = make_upstream("http://backend1");
    let params = super::lb::p2c_ewma::P2cEwmaParams::default();

    crate::upstream::lb::p2c_ewma::update_ewma(&ewma_state, &upstream, 0.5, &params);
    let data = ewma_state.get(&upstream).unwrap();
    assert!((data.ewma - 0.5).abs() < 1e-9);
    assert_eq!(data.sample_count, 1);
    drop(data);

    crate::upstream::lb::p2c_ewma::update_ewma(&ewma_state, &upstream, 0.3, &params);
    let data = ewma_state.get(&upstream).unwrap();
    assert!((data.ewma - 0.4).abs() < 1e-9);
    assert_eq!(data.sample_count, 2);
    drop(data);
}

#[test]
fn test_p2c_ewma_warmup_transitions_to_ewma() {
    let ewma_state: EwmaStateMap = Arc::new(DashMap::new());
    let upstream = make_upstream("http://backend1");
    let params = super::lb::p2c_ewma::P2cEwmaParams::default();

    for _ in 0..10 {
        crate::upstream::lb::p2c_ewma::update_ewma(&ewma_state, &upstream, 0.1, &params);
    }
    {
        let data = ewma_state.get(&upstream).unwrap();
        assert_eq!(data.sample_count, 10);
        assert!((data.ewma - 0.1).abs() < 1e-9);
    }

    crate::upstream::lb::p2c_ewma::update_ewma(&ewma_state, &upstream, 1.0, &params);
    {
        let data = ewma_state.get(&upstream).unwrap();
        assert_eq!(data.sample_count, 11);
        let expected = params.alpha * 1.0 + (1.0 - params.alpha) * 0.1;
        assert!((data.ewma - expected).abs() < 1e-9);
    }
}

#[test]
fn test_p2c_ewma_default_ewma_for_unknown_backend() {
    let ewma_state: EwmaStateMap = Arc::new(DashMap::new());
    let upstream = make_upstream("http://unknown");
    let params = super::lb::p2c_ewma::P2cEwmaParams::default();

    let ewma = crate::upstream::lb::p2c_ewma::get_decayed_ewma(&ewma_state, &upstream, &params);
    assert!((ewma - params.default_ewma).abs() < 1e-9);
}

#[test]
fn test_p2c_ewma_compute_score() {
    let params = super::lb::p2c_ewma::P2cEwmaParams::default();
    let score = crate::upstream::lb::p2c_ewma::compute_score(0.5, 4, &params);
    // Expected: 0.5 + 4 * 0.5 = 0.5 + 2.0 = 2.5
    let expected = 0.5 + 4.0 * params.connection_penalty;
    assert!((score - expected).abs() < 1e-9);
}
