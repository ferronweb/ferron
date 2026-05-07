use super::*;

fn make_upstream(url: &str) -> UpstreamInner {
    UpstreamInner {
        proxy_to: url.to_string(),
        proxy_unix: None,
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
        let idx = select_backend_index(&algorithm, &backends, None);
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
    let counter = Arc::new(AtomicUsize::new(0));
    let algorithm = LoadBalancerAlgorithmInner::RoundRobin(counter);

    assert_eq!(select_backend_index(&algorithm, &backends, None), 0);
    assert_eq!(select_backend_index(&algorithm, &backends, None), 1);
    assert_eq!(select_backend_index(&algorithm, &backends, None), 2);
    assert_eq!(select_backend_index(&algorithm, &backends, None), 0);
}

#[test]
fn test_select_backend_index_least_connections() {
    let backends = vec![
        make_upstream("http://backend1"),
        make_upstream("http://backend2"),
    ];
    let conn_state: ConnectionsTrackState = Arc::new(RwLock::new(HashMap::new()));
    let algorithm = LoadBalancerAlgorithmInner::LeastConnections;

    let idx = select_backend_index(&algorithm, &backends, Some(&conn_state));
    assert!(idx < backends.len());

    let tracker1 = Arc::new(());
    conn_state
        .write()
        .insert(backends[0].clone(), tracker1.clone());
    let _clone1 = tracker1.clone();
    let _clone2 = tracker1.clone();

    let idx = select_backend_index(&algorithm, &backends, Some(&conn_state));
    assert_eq!(idx, 1);
}

#[test]
fn test_select_backend_index_two_random_choices() {
    let backends = vec![
        make_upstream("http://backend1"),
        make_upstream("http://backend2"),
        make_upstream("http://backend3"),
    ];
    let conn_state: ConnectionsTrackState = Arc::new(RwLock::new(HashMap::new()));
    let algorithm = LoadBalancerAlgorithmInner::TwoRandomChoices;

    for _ in 0..100 {
        let idx = select_backend_index(&algorithm, &backends, Some(&conn_state));
        assert!(idx < backends.len());
    }
}

#[test]
fn test_select_backend_single_backend() {
    let backends = vec![make_upstream("http://backend1")];
    let conn_state: ConnectionsTrackState = Arc::new(RwLock::new(HashMap::new()));
    let algorithm = LoadBalancerAlgorithmInner::TwoRandomChoices;

    let idx = select_backend_index(&algorithm, &backends, Some(&conn_state));
    assert_eq!(idx, 0);
}

#[test]
fn test_determine_proxy_to_no_upstreams() {
    let failed_backends: Arc<RwLock<TtlCache<UpstreamInner, u64>>> =
        Arc::new(RwLock::new(TtlCache::new(Duration::from_secs(60))));
    let algorithm = LoadBalancerAlgorithmInner::Random;

    let result = determine_proxy_to(&[], &failed_backends, false, 3, &algorithm, None, None, &[]);
    assert!(result.is_none());
}

#[test]
fn test_determine_proxy_to_single_backend() {
    let upstreams = vec![make_upstream("http://backend1")];
    let failed_backends: Arc<RwLock<TtlCache<UpstreamInner, u64>>> =
        Arc::new(RwLock::new(TtlCache::new(Duration::from_secs(60))));
    let algorithm = LoadBalancerAlgorithmInner::Random;
    let conn_state: ConnectionsTrackState = Arc::new(RwLock::new(HashMap::new()));

    let result = determine_proxy_to(
        &upstreams,
        &failed_backends,
        false,
        3,
        &algorithm,
        Some(&conn_state),
        None,
        &[],
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
        &[],
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
        &[],
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
        &[],
    );
    assert!(result.is_some());
}

#[test]
fn test_mark_backend_failure() {
    let failed_backends: Arc<RwLock<TtlCache<UpstreamInner, u64>>> =
        Arc::new(RwLock::new(TtlCache::new(Duration::from_secs(60))));
    let upstream = make_upstream("http://backend1");
    let mut metrics = crate::ProxyMetrics::new();

    mark_backend_failure(Arc::clone(&failed_backends), true, &upstream, &mut metrics);

    assert_eq!(metrics.unhealthy_backends.len(), 1);
    assert_eq!(failed_backends.read().get(&upstream), Some(1));

    mark_backend_failure(Arc::clone(&failed_backends), true, &upstream, &mut metrics);

    assert_eq!(failed_backends.read().get(&upstream), Some(2));
}

#[test]
fn test_mark_backend_failure_health_check_disabled() {
    let failed_backends: Arc<RwLock<TtlCache<UpstreamInner, u64>>> =
        Arc::new(RwLock::new(TtlCache::new(Duration::from_secs(60))));
    let upstream = make_upstream("http://backend1");
    let mut metrics = crate::ProxyMetrics::new();

    mark_backend_failure(Arc::clone(&failed_backends), false, &upstream, &mut metrics);

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
    use std::collections::HashMap;

    let upstreams = vec![
        make_upstream("http://backend1"),
        make_upstream("http://backend2"),
    ];
    let failed_backends: Arc<RwLock<TtlCache<UpstreamInner, u64>>> =
        Arc::new(RwLock::new(TtlCache::new(Duration::from_secs(60))));

    let health_check_state: HealthCheckStateMap = Arc::new(RwLock::new(HashMap::new()));
    {
        let mut states = health_check_state.write();
        states.insert(
            "http://backend1".to_string(),
            HealthCheckState {
                is_healthy: false,
                ..Default::default()
            },
        );
    }

    let algorithm = LoadBalancerAlgorithmInner::Random;

    let result = determine_proxy_to(
        &upstreams,
        &failed_backends,
        true,
        3,
        &algorithm,
        None,
        Some(&health_check_state),
        &[],
    );
    assert!(result.is_some());
    assert_eq!(result.unwrap().upstream.proxy_to, "http://backend2");
}

#[test]
fn test_determine_proxy_to_active_health_check_all_healthy() {
    use std::collections::HashMap;

    let upstreams = vec![
        make_upstream("http://backend1"),
        make_upstream("http://backend2"),
    ];
    let failed_backends: Arc<RwLock<TtlCache<UpstreamInner, u64>>> =
        Arc::new(RwLock::new(TtlCache::new(Duration::from_secs(60))));

    let health_check_state: HealthCheckStateMap = Arc::new(RwLock::new(HashMap::new()));
    let algorithm = LoadBalancerAlgorithmInner::Random;

    let result = determine_proxy_to(
        &upstreams,
        &failed_backends,
        true,
        3,
        &algorithm,
        None,
        Some(&health_check_state),
        &[],
    );
    assert!(result.is_some());
    let selected = result.unwrap();
    assert!(
        selected.upstream.proxy_to == "http://backend1"
            || selected.upstream.proxy_to == "http://backend2"
    );
}
