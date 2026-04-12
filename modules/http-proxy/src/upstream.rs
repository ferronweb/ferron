//! Upstream resolution and load balancing logic.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;

use crate::util::TtlCache;

/// Tracks health state per upstream URL/config combination.
///
/// Keyed by the proxy_to URL; stores the current health status,
/// consecutive counters, and last probe results.
pub type HealthCheckStateMap = Arc<RwLock<HashMap<String, HealthCheckState>>>;

/// Upstream connection key.
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct UpstreamInner {
    /// Target URL (e.g. `http://localhost:8080/path`).
    pub proxy_to: String,
    /// Optional Unix socket path.
    pub proxy_unix: Option<String>,
}

/// Proxy protocol version.
#[derive(Clone, Copy, Debug)]
pub enum ProxyHeader {
    /// HAProxy PROXY protocol v1.
    V1,
    /// HAProxy PROXY protocol v2.
    V2,
}

/// Load balancing algorithm.
#[derive(Clone, Copy, Debug, Default)]
pub enum LoadBalancerAlgorithm {
    /// Random selection.
    Random,
    /// Round-robin cycling.
    RoundRobin,
    /// Least active connections.
    LeastConnections,
    /// Pick two random, select less loaded.
    #[default]
    TwoRandomChoices,
}

/// Runtime load balancer state.
#[derive(Clone, Default)]
pub enum LoadBalancerAlgorithmInner {
    Random,
    RoundRobin(Arc<AtomicUsize>),
    #[default]
    LeastConnections,
    TwoRandomChoices,
}

impl From<LoadBalancerAlgorithm> for LoadBalancerAlgorithmInner {
    fn from(alg: LoadBalancerAlgorithm) -> Self {
        match alg {
            LoadBalancerAlgorithm::Random => Self::Random,
            LoadBalancerAlgorithm::RoundRobin => Self::RoundRobin(Arc::new(AtomicUsize::new(0))),
            LoadBalancerAlgorithm::LeastConnections => Self::LeastConnections,
            LoadBalancerAlgorithm::TwoRandomChoices => Self::TwoRandomChoices,
        }
    }
}

/// Shared connection tracking state for least-conn and two-random algorithms.
pub type ConnectionsTrackState = Arc<RwLock<HashMap<UpstreamInner, Arc<()>>>>;

/// HTTP method for active health checks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HealthCheckMethod {
    /// HTTP GET request.
    Get,
    /// HTTP HEAD request.
    Head,
}

impl HealthCheckMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            HealthCheckMethod::Get => "GET",
            HealthCheckMethod::Head => "HEAD",
        }
    }
}

/// Expected HTTP status codes for health check success.
#[derive(Clone, Debug)]
pub enum ExpectedStatusCodes {
    /// Match 2xx responses.
    Successful,
    /// Match 2xx and 3xx responses.
    SuccessfulOrRedirect,
    /// Match specific status code.
    Specific(u16),
    /// Match any status code in the list.
    Any(Vec<u16>),
    /// Match status codes in the range [start, end] inclusive.
    Range(u16, u16),
}

impl ExpectedStatusCodes {
    /// Check if a given status code matches.
    pub fn matches(&self, status: u16) -> bool {
        match self {
            ExpectedStatusCodes::Successful => (200..300).contains(&status),
            ExpectedStatusCodes::SuccessfulOrRedirect => (200..400).contains(&status),
            ExpectedStatusCodes::Specific(code) => status == *code,
            ExpectedStatusCodes::Any(codes) => codes.contains(&status),
            ExpectedStatusCodes::Range(start, end) => (*start..=*end).contains(&status),
        }
    }
}

/// Active health check configuration for an upstream.
#[derive(Clone, Debug)]
pub struct UpstreamHealthCheckConfig {
    /// Enable active health checks for this upstream.
    pub enabled: bool,
    /// HTTP method for probe requests (GET or HEAD).
    pub method: HealthCheckMethod,
    /// Endpoint to probe (e.g., `/health`).
    pub uri: String,
    /// Interval between probes.
    pub interval: Duration,
    /// Max wait for probe response.
    pub timeout: Duration,
    /// Expected HTTP status codes for success.
    pub expect_status: ExpectedStatusCodes,
    /// Max response time threshold. If set, mark unhealthy if response takes longer.
    pub response_time_threshold: Option<Duration>,
    /// Optional substring to match in response body (only for GET).
    pub body_match: Option<String>,
    /// Mark unhealthy after N consecutive failures.
    pub consecutive_fails: u64,
    /// Mark healthy after N consecutive successes when recovering.
    pub consecutive_passes: u64,
    /// Skip TLS certificate verification for HTTPS probes.
    pub no_verification: bool,
}

impl Default for UpstreamHealthCheckConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            method: HealthCheckMethod::Get,
            uri: "/health".to_string(),
            interval: Duration::from_secs(10),
            timeout: Duration::from_secs(5),
            expect_status: ExpectedStatusCodes::SuccessfulOrRedirect,
            response_time_threshold: None,
            body_match: None,
            consecutive_fails: 2,
            consecutive_passes: 2,
            no_verification: false,
        }
    }
}

/// Health check state for tracking probe results per upstream.
#[derive(Clone, Debug)]
pub struct HealthCheckState {
    /// Current health status: true = healthy, false = unhealthy.
    pub is_healthy: bool,
    /// Consecutive failure counter when unhealthy.
    pub consecutive_fail_count: u64,
    /// Consecutive success counter when recovering.
    pub consecutive_pass_count: u64,
    /// Last probe result status code (if available).
    pub last_probe_status: Option<u16>,
    /// Last probe error message (if any).
    pub last_probe_error: Option<String>,
    /// Timestamp of last successful probe.
    pub last_success_time: Option<std::time::SystemTime>,
    /// Timestamp of last failed probe.
    pub last_failure_time: Option<std::time::SystemTime>,
}

impl Default for HealthCheckState {
    fn default() -> Self {
        Self {
            is_healthy: true,
            consecutive_fail_count: 0,
            consecutive_pass_count: 0,
            last_probe_status: None,
            last_probe_error: None,
            last_success_time: None,
            last_failure_time: None,
        }
    }
}

/// Configured upstream backend.
#[derive(Clone, Debug)]
pub struct UpstreamConfig {
    /// Target URL.
    pub url: String,
    /// Optional Unix socket path.
    pub unix_socket: Option<String>,
    /// Per-upstream connection limit.
    pub limit: Option<usize>,
    /// Idle keep-alive timeout. Populated into `ProxyConfig::idle_timeout_map`
    /// during parsing for O(1) lookup at request time.
    #[allow(dead_code)]
    pub idle_timeout: Option<Duration>,
    /// Active health check configuration for this upstream.
    pub health_check_config: UpstreamHealthCheckConfig,
}

/// Data for an SRV-based upstream.
///
/// The DNS resolver and runtime handle are obtained lazily at resolution time
/// from the globally-captured secondary runtime handle.
#[cfg(feature = "srv-lookup")]
#[derive(Clone)]
pub struct SrvUpstreamData {
    /// SRV record name (e.g. `_http._tcp.example.com`).
    pub srv_name: String,
    /// Custom DNS servers (empty = use system resolver).
    pub dns_servers: Vec<std::net::IpAddr>,
    /// Per-upstream connection limit.
    pub limit: Option<usize>,
    /// Idle keep-alive timeout.
    #[allow(dead_code)]
    pub idle_timeout: Option<Duration>,
}

/// An upstream backend — either a static URL or an SRV record.
#[derive(Clone)]
pub enum Upstream {
    /// Static upstream with a fixed URL and configuration.
    Static(UpstreamConfig),
    /// SRV-based upstream resolved via DNS.
    #[cfg(feature = "srv-lookup")]
    Srv(SrvUpstreamData),
}

impl Upstream {
    /// Resolve this upstream to a list of concrete `UpstreamInner` entries.
    ///
    /// Static upstreams return themselves. SRV upstreams perform a DNS lookup
    /// on the secondary Tokio runtime, filter unhealthy backends, and perform
    /// weighted random selection within the highest-priority group.
    pub async fn resolve(
        &self,
        _failed_backends: Arc<RwLock<TtlCache<UpstreamInner, u64>>>,
        _health_check_max_fails: u64,
    ) -> Vec<UpstreamInner> {
        match self {
            Upstream::Static(cfg) => vec![UpstreamInner {
                proxy_to: cfg.url.clone(),
                proxy_unix: cfg.unix_socket.clone(),
            }],
            #[cfg(feature = "srv-lookup")]
            Upstream::Srv(srv_data) => {
                resolve_srv(srv_data, _failed_backends, _health_check_max_fails).await
            }
        }
    }
}

/// Resolve an SRV record to a list of upstream backends.
#[cfg(feature = "srv-lookup")]
async fn resolve_srv(
    srv_data: &SrvUpstreamData,
    failed_backends: Arc<RwLock<TtlCache<UpstreamInner, u64>>>,
    health_check_max_fails: u64,
) -> Vec<UpstreamInner> {
    use hickory_proto::xfer::Protocol;
    use hickory_resolver::config::{NameServerConfig, ResolverConfig};
    use hickory_resolver::TokioResolver;

    let srv_name = srv_data.srv_name.clone();
    let dns_servers = srv_data.dns_servers.clone();

    // Get the secondary runtime handle (captured globally during Module::start)
    let handle = match crate::try_get_secondary_runtime_handle() {
        Some(h) => h,
        None => {
            ferron_core::log_warn!("SRV resolution skipped — secondary runtime not yet available");
            return Vec::new();
        }
    };

    // Spawn SRV lookup on the secondary Tokio runtime
    let result = handle
        .spawn(async move {
            use hickory_proto::runtime::TokioRuntimeProvider;
            use hickory_resolver::name_server::TokioConnectionProvider;

            // Build resolver inside the spawned task (we're on the secondary runtime)
            let resolver = if !dns_servers.is_empty() {
                let mut resolver_config = ResolverConfig::new();
                for server in &dns_servers {
                    resolver_config.add_name_server(NameServerConfig {
                        socket_addr: std::net::SocketAddr::new(*server, 53),
                        protocol: Protocol::Udp,
                        tls_dns_name: None,
                        trust_negative_responses: false,
                        bind_addr: None,
                        http_endpoint: None,
                    });
                }
                TokioResolver::builder_with_config(
                    resolver_config,
                    TokioConnectionProvider::new(TokioRuntimeProvider::new()),
                )
                .build()
            } else {
                TokioResolver::builder_tokio()
                    .unwrap_or_else(|_| {
                        TokioResolver::builder_with_config(
                            ResolverConfig::default(),
                            TokioConnectionProvider::new(TokioRuntimeProvider::new()),
                        )
                    })
                    .build()
            };

            // Perform SRV lookup
            let srv_records = match resolver.srv_lookup(&srv_name).await {
                Ok(records) => records,
                Err(e) => {
                    ferron_core::log_warn!("SRV lookup failed for {}: {}", srv_name, e);
                    return Vec::new();
                }
            };

            // Parse the SRV records into upstream candidates
            let candidates: Vec<(UpstreamInner, u16, u16)> = srv_records
                .iter()
                .map(|srv| {
                    let target = srv.target().to_string();
                    let port = srv.port();

                    let proxy_to = format!("http://{}:{}", target.trim_end_matches('.'), port);
                    let upstream = UpstreamInner {
                        proxy_to,
                        proxy_unix: None,
                    };

                    (upstream, srv.weight(), srv.priority())
                })
                .collect();

            if candidates.is_empty() {
                return Vec::new();
            }

            // Filter out unhealthy backends
            let failed = failed_backends.read();
            let healthy: Vec<(UpstreamInner, u16, u16)> = candidates
                .into_iter()
                .filter(|(upstream, _, _)| {
                    failed
                        .get(upstream)
                        .is_none_or(|fails| fails <= health_check_max_fails)
                })
                .collect();
            drop(failed);

            if healthy.is_empty() {
                return Vec::new();
            }

            // Select the highest-priority group (lowest numeric value)
            let highest_priority = healthy
                .iter()
                .map(|(_, _, priority)| *priority)
                .min()
                .unwrap_or(0);

            let filtered: Vec<(UpstreamInner, u16)> = healthy
                .into_iter()
                .filter(|(_, _, priority)| *priority == highest_priority)
                .map(|(upstream, weight, _)| (upstream, weight))
                .collect();

            // Weighted random selection
            let cumulative_weight: u32 = filtered.iter().map(|(_, w)| *w as u32).sum();
            if cumulative_weight == 0 {
                return filtered.into_iter().map(|(u, _)| u).collect();
            }

            let mut random_weight = rand::random_range(0..cumulative_weight);
            for (upstream, weight) in filtered {
                if random_weight < weight as u32 {
                    return vec![upstream];
                }
                random_weight -= weight as u32;
            }

            Vec::new()
        })
        .await;

    result.unwrap_or_default()
}

/// Resolve all upstreams to a flat list of `UpstreamInner` entries.
///
/// For SRV upstreams, this performs DNS resolution. For static upstreams,
/// it returns them as-is.
pub async fn resolve_upstreams(
    upstreams: &[Upstream],
    failed_backends: Arc<RwLock<TtlCache<UpstreamInner, u64>>>,
    health_check_max_fails: u64,
) -> Vec<UpstreamInner> {
    let mut resolved = Vec::new();
    for upstream in upstreams {
        resolved.extend(
            upstream
                .resolve(Arc::clone(&failed_backends), health_check_max_fails)
                .await,
        );
    }
    resolved
}

/// Selects a backend index based on the load balancing algorithm.
///
/// For LeastConnections and TwoRandomChoices, also initializes the connection
/// tracker `Arc<()>` in the map if missing, so that the caller can simply
/// clone the existing entry without a second lock acquisition.
fn select_backend_index(
    load_balancer_algorithm: &LoadBalancerAlgorithmInner,
    backends: &[UpstreamInner],
    conn_state: Option<&ConnectionsTrackState>,
) -> usize {
    match load_balancer_algorithm {
        LoadBalancerAlgorithmInner::Random => rand::random_range(0..backends.len()),
        LoadBalancerAlgorithmInner::RoundRobin(counter) => {
            counter.fetch_add(1, Ordering::Relaxed) % backends.len()
        }
        LoadBalancerAlgorithmInner::LeastConnections => {
            let Some(conn_state) = conn_state else {
                return 0;
            };
            let mut min_indexes = Vec::new();
            let mut min_connections = None;
            for (index, upstream) in backends.iter().enumerate() {
                let connection_track_read = conn_state.read();
                let connection_count = if let Some(tracker) = connection_track_read.get(upstream) {
                    Arc::strong_count(tracker) - 1
                } else {
                    drop(connection_track_read);
                    conn_state.write().insert(upstream.clone(), Arc::new(()));
                    0
                };
                if min_connections.is_none_or(|min| connection_count < min) {
                    min_indexes = vec![index];
                    min_connections = Some(connection_count);
                } else if min_connections == Some(connection_count) {
                    min_indexes.push(index);
                }
            }
            match min_indexes.len() {
                0 => 0,
                1 => min_indexes[0],
                _ => min_indexes[rand::random_range(0..min_indexes.len())],
            }
        }
        LoadBalancerAlgorithmInner::TwoRandomChoices => {
            let Some(conn_state) = conn_state else {
                return rand::random_range(0..backends.len());
            };
            if backends.len() < 2 {
                // Initialize tracker for single backend
                let read = conn_state.read();
                if read.get(&backends[0]).is_none() {
                    drop(read);
                    conn_state.write().insert(backends[0].clone(), Arc::new(()));
                }
                return 0;
            }
            let idx1 = rand::random_range(0..backends.len());
            let mut idx2 = rand::random_range(0..backends.len() - 1);
            if idx2 >= idx1 {
                idx2 += 1;
            }

            // Get count for first backend
            let (count1, _read_dropped) = {
                let read = conn_state.read();
                if let Some(t) = read.get(&backends[idx1]) {
                    (Arc::strong_count(t) - 1, false)
                } else {
                    drop(read);
                    conn_state
                        .write()
                        .insert(backends[idx1].clone(), Arc::new(()));
                    (0, true)
                }
            };

            // Get count for second backend
            let count2 = {
                let read = conn_state.read();
                if let Some(t) = read.get(&backends[idx2]) {
                    Arc::strong_count(t) - 1
                } else {
                    drop(read);
                    conn_state
                        .write()
                        .insert(backends[idx2].clone(), Arc::new(()));
                    0
                }
            };

            if count2 >= count1 {
                idx1
            } else {
                idx2
            }
        }
    }
}

/// Result of backend selection: the upstream and its connection tracker.
pub struct SelectedBackend {
    /// The selected upstream.
    pub upstream: UpstreamInner,
    /// Connection tracker for LeastConnections/TwoRandomChoices.
    /// `None` for Random/RoundRobin algorithms.
    pub tracker: Option<Arc<()>>,
}

/// Determines which backend server to proxy the request to.
///
/// Returns the selected upstream and its connection tracker (if applicable).
/// Filters out unhealthy backends when health checking is enabled, consulting
/// both the passive failure cache and active health check state.
pub fn determine_proxy_to(
    upstreams: &[UpstreamInner],
    failed_backends: &RwLock<TtlCache<UpstreamInner, u64>>,
    health_check_enabled: bool,
    health_check_max_fails: u64,
    algorithm: &LoadBalancerAlgorithmInner,
    conn_state: Option<&ConnectionsTrackState>,
    health_check_state: Option<&HealthCheckStateMap>,
    selected_backends: &[UpstreamInner],
) -> Option<SelectedBackend> {
    if upstreams.is_empty() {
        return None;
    }

    // Build a mutable copy of healthy backends for the selection loop
    let mut healthy: Vec<UpstreamInner> = {
        let failed = if health_check_enabled {
            Some(failed_backends.read())
        } else {
            None
        };
        upstreams
            .iter()
            .filter(|u| {
                // Check passive failure cache
                let not_failed = failed.as_ref().is_none_or(|failed| {
                    failed
                        .get(*u)
                        .is_none_or(|fails| fails <= health_check_max_fails)
                });

                // Check active health check state
                let active_healthy = if let Some(state_map) = health_check_state {
                    crate::health_check::is_upstream_healthy(state_map, &u.proxy_to)
                } else {
                    true
                };

                // Check if backend is already selected
                let not_selected = !selected_backends.contains(u);

                not_failed && active_healthy && not_selected
            })
            .cloned()
            .collect()
    };

    if healthy.is_empty() {
        return None;
    }

    if healthy.len() == 1 {
        // Single backend — initialize tracker if needed
        let tracker = initialize_tracker(conn_state, &healthy[0]);
        return Some(SelectedBackend {
            upstream: healthy.remove(0),
            tracker,
        });
    }

    // Selection loop: skip unhealthy backends
    loop {
        if healthy.is_empty() {
            return None;
        }
        let index = select_backend_index(algorithm, &healthy, conn_state);
        let upstream = healthy.remove(index);

        if health_check_enabled {
            let failed = failed_backends.read();
            if let Some(fails) = failed.get(&upstream) {
                if fails > health_check_max_fails {
                    continue; // Skip unhealthy, try next
                }
            }
        }

        // Get the tracker (already initialized by select_backend_index)
        let tracker = get_tracker(conn_state, &upstream);
        return Some(SelectedBackend { upstream, tracker });
    }
}

/// Get or create the connection tracker for an upstream.
fn initialize_tracker(
    conn_state: Option<&ConnectionsTrackState>,
    upstream: &UpstreamInner,
) -> Option<Arc<()>> {
    let conn_state = conn_state?;
    let read = conn_state.read();
    if read.get(upstream).is_some() {
        return None; // Tracker already exists, caller will clone it
    }
    drop(read);
    conn_state.write().insert(upstream.clone(), Arc::new(()));
    None
}

/// Clone an existing connection tracker for an upstream.
fn get_tracker(
    conn_state: Option<&ConnectionsTrackState>,
    upstream: &UpstreamInner,
) -> Option<Arc<()>> {
    let conn_state = conn_state?;
    conn_state.read().get(upstream).map(Arc::clone)
}

/// Mark a backend as failed.
pub fn mark_backend_failure(
    failed_backends: Arc<RwLock<TtlCache<UpstreamInner, u64>>>,
    health_check_enabled: bool,
    upstream: &UpstreamInner,
    metrics: &mut crate::ProxyMetrics,
) {
    if !health_check_enabled {
        return;
    }
    metrics.unhealthy_backends.push(upstream.clone());
    let mut failed = failed_backends.write();
    let current = failed.get(upstream).unwrap_or(0);
    failed.insert(upstream.clone(), current + 1);
}

#[cfg(test)]
mod tests {
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

        // Random should return a valid index
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

        // Should cycle through backends
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

        // With no connections, should return first backend (both have 0 connections)
        let idx = select_backend_index(&algorithm, &backends, Some(&conn_state));
        assert!(idx < backends.len());

        // Simulate more active connections on backend1 by cloning the tracker Arc
        // The algorithm uses Arc::strong_count(tracker) - 1 to count connections
        let tracker1 = Arc::new(());
        conn_state
            .write()
            .insert(backends[0].clone(), tracker1.clone());
        // Clone to simulate 2 active connections (strong_count = 3, so 3-1 = 2)
        let _clone1 = tracker1.clone();
        let _clone2 = tracker1.clone();

        // backend2 has 0 connections (not in map), backend1 has 2
        // Should prefer backend2 (less connections)
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

        // Should return valid indices
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

        let result =
            determine_proxy_to(&[], &failed_backends, false, 3, &algorithm, None, None, &[]);
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

        // Mark backend1 as unhealthy (exceeds max_fails)
        {
            let mut failed = failed_backends.write();
            failed.insert(make_upstream("http://backend1"), 5);
        }

        let algorithm = LoadBalancerAlgorithmInner::Random;

        // With health check enabled, should only select backend2
        let result = determine_proxy_to(
            &upstreams,
            &failed_backends,
            true,
            3, // max_fails
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

        // Mark all backends as unhealthy
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
            3, // max_fails
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

        // Mark backend1 as unhealthy, but health check is disabled so it should still be selected
        {
            let mut failed = failed_backends.write();
            failed.insert(make_upstream("http://backend1"), 100);
        }

        let algorithm = LoadBalancerAlgorithmInner::Random;

        let result = determine_proxy_to(
            &upstreams,
            &failed_backends,
            false, // health check disabled
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

        // Second failure
        mark_backend_failure(Arc::clone(&failed_backends), true, &upstream, &mut metrics);

        assert_eq!(failed_backends.read().get(&upstream), Some(2));
    }

    #[test]
    fn test_mark_backend_failure_health_check_disabled() {
        let failed_backends: Arc<RwLock<TtlCache<UpstreamInner, u64>>> =
            Arc::new(RwLock::new(TtlCache::new(Duration::from_secs(60))));
        let upstream = make_upstream("http://backend1");
        let mut metrics = crate::ProxyMetrics::new();

        mark_backend_failure(
            Arc::clone(&failed_backends),
            false, // health check disabled
            &upstream,
            &mut metrics,
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
        use std::collections::HashMap;

        let upstreams = vec![
            make_upstream("http://backend1"),
            make_upstream("http://backend2"),
        ];
        let failed_backends: Arc<RwLock<TtlCache<UpstreamInner, u64>>> =
            Arc::new(RwLock::new(TtlCache::new(Duration::from_secs(60))));

        // Create active health check state with backend1 marked unhealthy
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

        // With health check enabled and active state, should only select backend2
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

        // All backends healthy
        let health_check_state: HealthCheckStateMap = Arc::new(RwLock::new(HashMap::new()));

        let algorithm = LoadBalancerAlgorithmInner::Random;

        // Should select one of the healthy backends
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
}
