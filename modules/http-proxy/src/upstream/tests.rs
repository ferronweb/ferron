use std::{sync::Arc, time::Duration};

use dashmap::DashMap;
use parking_lot::RwLock;

use crate::types::affinity::{AffinityType, CookieAffinityConfig};
use crate::types::health::{HealthCheckState, HealthCheckStateMap};
use crate::types::lb::LoadBalancerAlgorithm;
use crate::types::upstream::UpstreamInner;
use crate::types::ConnectionsTrackState;
use crate::upstream::affinity::resolve_affinity_index;
use crate::upstream::lb::{
    selector::select_backend_index, ConsistentHashRing, LoadBalancerAlgorithmInner,
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
        make_upstream("http://backend1"),
        make_upstream("http://backend2"),
        make_upstream("http://backend3"),
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
        make_upstream("http://backend1"),
        make_upstream("http://backend2"),
        make_upstream("http://backend3"),
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
        make_upstream("http://backend1"),
        make_upstream("http://backend2"),
    ];
    let conn_state: ConnectionsTrackState = Arc::new(DashMap::new());
    let algorithm = LoadBalancerAlgorithmInner::LeastConnections;

    let idx = select_backend_index(&algorithm, &backends, Some(&conn_state), None);
    assert!(idx < backends.len());

    let tracker1 = Arc::new(());
    conn_state.insert(backends[0].clone(), tracker1.clone());
    let _clone1 = tracker1.clone();
    let _clone2 = tracker1.clone();

    let idx = select_backend_index(&algorithm, &backends, Some(&conn_state), None);
    assert_eq!(idx, 1);
}

#[test]
fn test_select_backend_index_two_random_choices() {
    let backends = vec![
        make_upstream("http://backend1"),
        make_upstream("http://backend2"),
        make_upstream("http://backend3"),
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
    let backends = vec![make_upstream("http://backend1")];
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
        &crate::config::CircuitBreakerConfig::default(),
        None,
        &[],
        None,
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
        &crate::config::CircuitBreakerConfig::default(),
        None,
        &[],
        None,
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
        &crate::config::CircuitBreakerConfig::default(),
        None,
        &[],
        None,
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
        &crate::config::CircuitBreakerConfig::default(),
        None,
        &[],
        None,
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
        &crate::config::CircuitBreakerConfig::default(),
        None,
        &[],
        None,
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
        Some(&health_check_state),
        &crate::config::CircuitBreakerConfig::default(),
        None,
        &[],
        None,
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
        Some(&health_check_state),
        &crate::config::CircuitBreakerConfig::default(),
        None,
        &[],
        None,
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
        make_upstream_with_weight("http://backend1", 1),
        make_upstream_with_weight("http://backend2", 1),
        make_upstream_with_weight("http://backend3", 1),
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
        make_upstream_with_weight("http://backend1", 5),
        make_upstream_with_weight("http://backend2", 1),
        make_upstream_with_weight("http://backend3", 1),
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
        make_upstream_with_weight("http://backend1", 5),
        make_upstream_with_weight("http://backend2", 1),
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
    let backends = vec![make_upstream_with_weight("http://backend1", 10)];
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
fn test_select_backend_index_consistent_hash() {
    let backends = vec![
        make_upstream("http://backend1"),
        make_upstream("http://backend2"),
        make_upstream("http://backend3"),
    ];
    let ring = Arc::new(RwLock::new(ConsistentHashRing::new(&backends)));
    let algorithm = LoadBalancerAlgorithmInner::ConsistentHash(ring);

    // Same key should always select the same backend
    let key = b"consistent-key";
    let idx1 = select_backend_index(&algorithm, &backends, None, Some(key));
    let idx2 = select_backend_index(&algorithm, &backends, None, Some(key));
    assert_eq!(idx1, idx2);
    assert!(idx1 < backends.len());
}

#[test]
fn test_backend_affinity_id() {
    let backend = make_upstream("http://backend1");
    let id = backend_affinity_id(&backend);

    // Should be a 16-character hex string
    assert_eq!(id.len(), 16);
    assert!(id.chars().all(|c| c.is_ascii_hexdigit()));

    // Same backend should always produce the same ID
    let id2 = backend_affinity_id(&backend);
    assert_eq!(id, id2);

    // Different backends should produce different IDs
    let backend2 = make_upstream("http://backend2");
    let id3 = backend_affinity_id(&backend2);
    assert_ne!(id, id3);
}

#[test]
fn test_resolve_affinity_index_cookie_match() {
    let backends = vec![
        make_upstream("http://backend1"),
        make_upstream("http://backend2"),
    ];
    let affinity_type = AffinityType::Cookie(CookieAffinityConfig::default());

    // Use the affinity ID of backend1
    let key = backend_affinity_id(&backends[0]);
    let idx = resolve_affinity_index(
        &affinity_type,
        key.as_bytes(),
        &backends,
        &LoadBalancerAlgorithmInner::Random,
    );
    assert_eq!(idx, Some(0));
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
        &LoadBalancerAlgorithmInner::Random,
    );
    assert!(idx.is_some());
    assert!(idx.unwrap() < backends.len());

    // Same IP should always map to the same backend
    let idx2 = resolve_affinity_index(
        &affinity_type,
        key,
        &backends,
        &LoadBalancerAlgorithmInner::Random,
    );
    assert_eq!(idx, idx2);
}

#[test]
fn test_determine_proxy_to_with_affinity() {
    let upstreams = vec![
        make_upstream("http://backend1"),
        make_upstream("http://backend2"),
    ];
    let failed_backends: Arc<RwLock<TtlCache<UpstreamInner, u64>>> =
        Arc::new(RwLock::new(TtlCache::new(Duration::from_secs(60))));
    let algorithm = LoadBalancerAlgorithmInner::Random;

    // With affinity index 1, should select backend2
    let result = determine_proxy_to(
        &upstreams,
        &failed_backends,
        false,
        3,
        &algorithm,
        None,
        None,
        &crate::config::CircuitBreakerConfig::default(),
        None,
        &[],
        Some(1),
        &ferron_observability::CompositeEventSink::new(vec![]),
    );
    assert!(result.is_some());
    assert_eq!(result.unwrap().upstream.proxy_to, "http://backend2");
}

#[test]
fn test_determine_proxy_to_affinity_out_of_range() {
    let upstreams = vec![
        make_upstream("http://backend1"),
        make_upstream("http://backend2"),
    ];
    let failed_backends: Arc<RwLock<TtlCache<UpstreamInner, u64>>> =
        Arc::new(RwLock::new(TtlCache::new(Duration::from_secs(60))));
    let algorithm = LoadBalancerAlgorithmInner::Random;

    // With affinity index out of range, should fall back to algorithm
    let result = determine_proxy_to(
        &upstreams,
        &failed_backends,
        false,
        3,
        &algorithm,
        None,
        None,
        &crate::config::CircuitBreakerConfig::default(),
        None,
        &[],
        Some(10),
        &ferron_observability::CompositeEventSink::new(vec![]),
    );
    assert!(result.is_some());
}
