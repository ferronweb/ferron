//! Upstream resolution and load balancing logic.

use std::collections::{HashMap, VecDeque};
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use http::header::HeaderName;
use parking_lot::{Mutex, RwLock};

use crate::config::CircuitBreakerConfig;
use crate::util::TtlCache;

/// Tracks health state per upstream URL/config combination.
///
/// Keyed by the proxy_to URL; stores the current health status,
/// consecutive counters, and last probe results.
pub type HealthCheckStateMap = Arc<RwLock<HashMap<String, HealthCheckState>>>;

/// Tracks circuit breaker state per upstream URL/config combination.
pub(crate) type CircuitBreakerStateMap = Arc<RwLock<HashMap<UpstreamInner, CircuitBreakerState>>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CircuitBreakerStatus {
    Closed,
    Open,
    HalfOpen,
}

#[derive(Clone, Debug)]
pub(crate) struct CircuitBreakerState {
    pub recent_failures: VecDeque<Instant>,
    pub status: CircuitBreakerStatus,
    pub opened_at: Option<Instant>,
    pub half_open_in_flight: bool,
    pub half_open_pass_count: u64,
}

impl Default for CircuitBreakerState {
    fn default() -> Self {
        Self {
            recent_failures: VecDeque::new(),
            status: CircuitBreakerStatus::Closed,
            opened_at: None,
            half_open_in_flight: false,
            half_open_pass_count: 0,
        }
    }
}

/// Upstream connection key.
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct UpstreamInner {
    /// Target URL (e.g. `http://localhost:8080/path`).
    pub proxy_to: String,
    /// Optional Unix socket path.
    pub proxy_unix: Option<String>,
    /// Weight for weighted load balancing algorithms (default 1).
    pub weight: u32,
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
    /// Smooth weighted round-robin.
    WeightedRoundRobin,
    /// Consistent hashing based on request key.
    ConsistentHash,
}

/// State for smooth weighted round-robin load balancing.
///
/// Uses Nginx's smooth weighted round-robin algorithm:
/// 1. Add each backend's weight to its current effective weight
/// 2. Select the backend with the highest effective weight
/// 3. Subtract total weight from the selected backend's effective weight
///
/// This ensures proportional distribution over time while avoiding bursts.
#[derive(Clone, Debug)]
pub struct WeightedRoundRobinState {
    /// Current effective weights for each backend position.
    /// Resized dynamically to match the active backend count.
    pub current_weights: Vec<i64>,
}

impl WeightedRoundRobinState {
    /// Create a new empty state.
    pub fn new() -> Self {
        Self {
            current_weights: Vec::new(),
        }
    }

    /// Select the next backend index using smooth weighted round-robin.
    ///
    /// The `weights` slice provides the configured weight for each backend
    /// at the corresponding index. The internal `current_weights` vector
    /// is resized automatically if the backend count changes.
    ///
    /// Returns the index of the selected backend.
    pub fn next(&mut self, weights: &[u32]) -> usize {
        let n = weights.len();
        if n == 0 {
            return 0;
        }

        // Resize current_weights if backend count changed
        if self.current_weights.len() != n {
            self.current_weights.resize(n, 0);
        }

        // Calculate total weight
        let total_weight: i64 = weights.iter().map(|w| *w as i64).sum();

        let mut best_index = 0;
        let mut best_weight = i64::MIN;

        // Step 1: Add each backend's weight to its current effective weight
        // Step 2: Find the backend with the highest effective weight
        for (i, weight) in weights.iter().enumerate() {
            self.current_weights[i] += *weight as i64;
            if self.current_weights[i] > best_weight {
                best_weight = self.current_weights[i];
                best_index = i;
            }
        }

        // Step 3: Subtract total weight from the selected backend's effective weight
        self.current_weights[best_index] -= total_weight;

        best_index
    }
}

/// Runtime load balancer state.
#[derive(Clone, Default)]
pub enum LoadBalancerAlgorithmInner {
    Random,
    RoundRobin(Arc<AtomicUsize>),
    #[default]
    LeastConnections,
    TwoRandomChoices,
    WeightedRoundRobin(Arc<Mutex<WeightedRoundRobinState>>),
    ConsistentHash(Arc<RwLock<ConsistentHashRing>>),
}

/// SameSite cookie attribute mode.
#[derive(Clone, Copy, Debug, Default)]
pub enum SameSiteMode {
    Strict,
    #[default]
    Lax,
    None,
}

impl SameSiteMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            SameSiteMode::Strict => "Strict",
            SameSiteMode::Lax => "Lax",
            SameSiteMode::None => "None",
        }
    }
}

/// Cookie-based session affinity configuration.
#[derive(Clone, Debug)]
pub struct CookieAffinityConfig {
    pub name: String,
    pub ttl: Option<Duration>,
    pub path: String,
    pub domain: Option<String>,
    pub secure: bool,
    pub httponly: bool,
    pub samesite: SameSiteMode,
}

impl Default for CookieAffinityConfig {
    fn default() -> Self {
        Self {
            name: "ferron_sticky".to_string(),
            ttl: None,
            path: "/".to_string(),
            domain: None,
            secure: false,
            httponly: true,
            samesite: SameSiteMode::Lax,
        }
    }
}

/// Hash method for hash-based affinity.
#[derive(Clone, Copy, Debug, Default)]
pub enum HashMethod {
    #[default]
    Consistent,
    Modulus,
}

/// Session affinity type configuration.
#[derive(Clone, Debug)]
pub enum AffinityType {
    Cookie(CookieAffinityConfig),
    Header(HeaderName),
    Ip,
    Hash {
        variable: String,
        #[allow(dead_code)]
        method: HashMethod,
    },
}

/// Session affinity configuration.
#[derive(Clone, Debug)]
pub struct AffinityConfig {
    pub affinity_type: AffinityType,
}

/// Ketama-style consistent hash ring for backend selection.
#[derive(Clone, Debug)]
pub struct ConsistentHashRing {
    nodes: Vec<(u64, usize)>,
    backend_count: usize,
}

impl ConsistentHashRing {
    const VNODES_PER_BACKEND: usize = 160;

    pub fn new(backends: &[UpstreamInner]) -> Self {
        let nodes = Self::build_nodes(backends);
        Self {
            nodes,
            backend_count: backends.len(),
        }
    }

    fn build_nodes(backends: &[UpstreamInner]) -> Vec<(u64, usize)> {
        let mut nodes = Vec::with_capacity(backends.len() * Self::VNODES_PER_BACKEND);

        for (idx, backend) in backends.iter().enumerate() {
            for vnode in 0..Self::VNODES_PER_BACKEND {
                let key = format!("{}#{}", backend.proxy_to, vnode);
                let mut h = get_ahasher();
                h.write(key.as_bytes());
                let hash = h.finish();
                nodes.push((hash, idx));
            }
        }

        nodes.sort_by_key(|&(hash, _)| hash);
        nodes
    }

    pub fn get(&self, key: &[u8]) -> Option<usize> {
        if self.nodes.is_empty() {
            return None;
        }

        let mut h = get_ahasher();
        h.write(key);
        let hash = h.finish();

        match self.nodes.binary_search_by_key(&hash, |(h, _)| *h) {
            Ok(idx) => Some(self.nodes[idx].1),
            Err(idx) => {
                if idx < self.nodes.len() {
                    Some(self.nodes[idx].1)
                } else {
                    Some(self.nodes[0].1)
                }
            }
        }
    }

    pub fn needs_rebuild(&self, backend_count: usize) -> bool {
        self.backend_count != backend_count
    }

    pub fn rebuild(&mut self, backends: &[UpstreamInner]) {
        self.nodes = Self::build_nodes(backends);
        self.backend_count = backends.len();
    }
}

impl From<LoadBalancerAlgorithm> for LoadBalancerAlgorithmInner {
    fn from(alg: LoadBalancerAlgorithm) -> Self {
        match alg {
            LoadBalancerAlgorithm::Random => Self::Random,
            LoadBalancerAlgorithm::RoundRobin => Self::RoundRobin(Arc::new(AtomicUsize::new(0))),
            LoadBalancerAlgorithm::LeastConnections => Self::LeastConnections,
            LoadBalancerAlgorithm::TwoRandomChoices => Self::TwoRandomChoices,
            LoadBalancerAlgorithm::WeightedRoundRobin => {
                Self::WeightedRoundRobin(Arc::new(Mutex::new(WeightedRoundRobinState::new())))
            }
            LoadBalancerAlgorithm::ConsistentHash => {
                Self::ConsistentHash(Arc::new(RwLock::new(ConsistentHashRing::new(&[]))))
            }
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
    /// Weight for weighted load balancing algorithms (default 1).
    pub weight: u32,
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
    /// Weight for weighted load balancing algorithms (default 1).
    pub weight: u32,
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
                weight: cfg.weight,
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
    use hickory_resolver::config::{NameServerConfig, ResolverConfig};
    use hickory_resolver::TokioResolver;

    let srv_name = srv_data.srv_name.clone();
    let dns_servers = srv_data.dns_servers.clone();
    let weight = srv_data.weight;

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
            use hickory_resolver::net::runtime::TokioRuntimeProvider;

            // Build resolver inside the spawned task (we're on the secondary runtime)
            let resolver_result = if !dns_servers.is_empty() {
                let mut resolver_config = ResolverConfig::default();
                for server in &dns_servers {
                    resolver_config.add_name_server(NameServerConfig::udp(*server));
                }
                TokioResolver::builder_with_config(resolver_config, TokioRuntimeProvider::new())
                    .build()
            } else {
                TokioResolver::builder_tokio()
                    .unwrap_or_else(|_| {
                        TokioResolver::builder_with_config(
                            ResolverConfig::default(),
                            TokioRuntimeProvider::new(),
                        )
                    })
                    .build()
            };
            let resolver = match resolver_result {
                Ok(resolver) => resolver,
                Err(e) => {
                    ferron_core::log_warn!("Failed to create resolver: {}", e);
                    return Vec::new();
                }
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
                .answers()
                .iter()
                .filter_map(|record| {
                    let srv = match &record.data {
                        hickory_proto::rr::RData::SRV(srv) => srv,
                        _ => return None,
                    };

                    let target = srv.target.to_string();
                    let port = srv.port;

                    let proxy_to = format!("http://{}:{}", target.trim_end_matches('.'), port);
                    let upstream = UpstreamInner {
                        proxy_to,
                        proxy_unix: None,
                        weight,
                    };

                    Some((upstream, srv.weight, srv.priority))
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
///
/// For ConsistentHash, `hash_key` must be provided.
fn select_backend_index(
    load_balancer_algorithm: &LoadBalancerAlgorithmInner,
    backends: &[UpstreamInner],
    conn_state: Option<&ConnectionsTrackState>,
    hash_key: Option<&[u8]>,
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
        LoadBalancerAlgorithmInner::WeightedRoundRobin(state) => {
            let weights: Vec<u32> = backends.iter().map(|b| b.weight).collect();
            let mut guard = state.lock();
            guard.next(&weights)
        }
        LoadBalancerAlgorithmInner::ConsistentHash(ring) => {
            let key = hash_key.unwrap_or(b"");
            let mut guard = ring.write();
            if guard.needs_rebuild(backends.len()) {
                guard.rebuild(backends);
            }
            guard.get(key).unwrap_or(0)
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
///
/// If `affinity_index` is provided, that backend is tried first. If it is
/// healthy, it is selected regardless of the load balancing algorithm.
/// Otherwise, the algorithm is used to select from the remaining backends.
#[allow(clippy::too_many_arguments)]
pub fn determine_proxy_to(
    upstreams: &[UpstreamInner],
    failed_backends: &RwLock<TtlCache<UpstreamInner, u64>>,
    health_check_enabled: bool,
    health_check_max_fails: u64,
    algorithm: &LoadBalancerAlgorithmInner,
    conn_state: Option<&ConnectionsTrackState>,
    health_check_state: Option<&HealthCheckStateMap>,
    circuit_breaker: &CircuitBreakerConfig,
    circuit_breaker_state: Option<&CircuitBreakerStateMap>,
    selected_backends: &[UpstreamInner],
    affinity_index: Option<usize>,
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

    let mut affinity_index = affinity_index;
    loop {
        if healthy.is_empty() {
            return None;
        }

        let index = if let Some(idx) = affinity_index.take() {
            if idx < healthy.len() {
                idx
            } else if healthy.len() == 1 {
                0
            } else {
                select_backend_index(algorithm, &healthy, conn_state, None)
            }
        } else if healthy.len() == 1 {
            0
        } else {
            select_backend_index(algorithm, &healthy, conn_state, None)
        };
        let upstream = healthy.remove(index);

        if !try_acquire_circuit_breaker_slot(circuit_breaker_state, circuit_breaker, &upstream) {
            continue;
        }

        if health_check_enabled {
            let failed = failed_backends.read();
            if let Some(fails) = failed.get(&upstream) {
                if fails > health_check_max_fails {
                    continue; // Skip unhealthy, try next
                }
            }
        }

        // Get the tracker (already initialized by select_backend_index)
        initialize_tracker(conn_state, &upstream);
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

/// Record a transport-level backend failure.
pub fn record_backend_transport_failure(
    failed_backends: Arc<RwLock<TtlCache<UpstreamInner, u64>>>,
    passive_check_enabled: bool,
    circuit_breaker_state: Option<&CircuitBreakerStateMap>,
    circuit_breaker: &CircuitBreakerConfig,
    upstream: &UpstreamInner,
    metrics: &mut crate::ProxyMetrics,
) {
    if passive_check_enabled {
        metrics.unhealthy_backends.push(upstream.clone());
        let mut failed = failed_backends.write();
        let current = failed.get(upstream).unwrap_or(0);
        failed.insert(upstream.clone(), current + 1);
    }

    if record_circuit_breaker_failure(circuit_breaker_state, circuit_breaker, upstream) {
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
) {
    let should_open = if is_circuit_breaker_failure_status(status) {
        record_circuit_breaker_failure(circuit_breaker_state, circuit_breaker, upstream)
    } else {
        record_circuit_breaker_success(circuit_breaker_state, circuit_breaker, upstream);
        false
    };

    if should_open {
        metrics
            .circuit_breaker_unhealthy_backends
            .push(upstream.clone());
    }
}

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

    let states = circuit_breaker_state.read();
    let Some(state) = states.get(upstream) else {
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

fn try_acquire_circuit_breaker_slot(
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

    let mut states = circuit_breaker_state.write();
    let state = states.entry(upstream.clone()).or_default();

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
            ferron_core::log_info!(
                "Upstream {} circuit transitioned to half-open",
                upstream.proxy_to
            );
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
) -> bool {
    if !circuit_breaker.enabled {
        return false;
    }

    let Some(circuit_breaker_state) = circuit_breaker_state else {
        return false;
    };

    let now = Instant::now();
    let mut states = circuit_breaker_state.write();
    let state = states.entry(upstream.clone()).or_default();

    match state.status {
        CircuitBreakerStatus::HalfOpen => {
            state.half_open_in_flight = false;
            state.half_open_pass_count = 0;
            state.recent_failures.clear();
            state.status = CircuitBreakerStatus::Open;
            state.opened_at = Some(now);
            ferron_core::log_warn!(
                "Upstream {} circuit reopened after a half-open trial failure",
                upstream.proxy_to
            );
            true
        }
        CircuitBreakerStatus::Open => {
            state.opened_at = Some(now);
            false
        }
        CircuitBreakerStatus::Closed => {
            prune_circuit_breaker_failures(state, circuit_breaker.window, now);
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
) {
    if !circuit_breaker.enabled {
        return;
    }

    let Some(circuit_breaker_state) = circuit_breaker_state else {
        return;
    };

    let mut states = circuit_breaker_state.write();
    let Some(state) = states.get_mut(upstream) else {
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
        ferron_core::log_info!(
            "Upstream {} circuit closed after {} successful half-open request(s)",
            upstream.proxy_to,
            circuit_breaker.consecutive_passes
        );
    }
}

fn prune_circuit_breaker_failures(state: &mut CircuitBreakerState, window: Duration, now: Instant) {
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

/// Resolve an affinity key to a backend index.
///
/// For cookie and header affinity, the key is a backend identifier
/// (hash of the upstream URL). For IP and hash affinity, the key
/// is used directly with the consistent hash ring.
pub fn resolve_affinity_index(
    affinity_type: &AffinityType,
    affinity_key: &[u8],
    backends: &[UpstreamInner],
    algorithm: &LoadBalancerAlgorithmInner,
) -> Option<usize> {
    if backends.is_empty() {
        return None;
    }

    match affinity_type {
        AffinityType::Cookie(_) | AffinityType::Header(_) => {
            // For cookie/header affinity, the key is a backend identifier.
            // We try to match it against each backend's identifier.
            let key_str = std::str::from_utf8(affinity_key).ok()?;
            backends
                .iter()
                .position(|b| backend_affinity_id(b) == key_str)
        }
        AffinityType::Ip | AffinityType::Hash { .. } => {
            // For IP and hash affinity, use consistent hashing.
            let ring = match algorithm {
                LoadBalancerAlgorithmInner::ConsistentHash(ring) => ring,
                _ => {
                    // Fall back to simple modulus hashing
                    let mut h = get_ahasher();
                    h.write(affinity_key);
                    let hash = h.finish();
                    return Some((hash as usize) % backends.len());
                }
            };
            let guard = ring.read();
            guard.get(affinity_key)
        }
    }
}

/// Generate a short affinity identifier for a backend.
///
/// Uses the first 8 hex characters of the upstream URL's ahash.
pub fn backend_affinity_id(backend: &UpstreamInner) -> String {
    let mut h = get_ahasher();
    h.write(backend.proxy_to.as_bytes());
    let hash = h.finish();
    format!("{hash:016x}")
}

/// Returns an [`ahash::AHasher`] with a consistent seed.
///
/// This is used for deterministic hashing of affinity keys,
/// so that the same key always maps to the same backend.
#[inline]
fn get_ahasher() -> ahash::AHasher {
    // Hard-coded seed values to ensure consistent hashing across deployments.
    ahash::RandomState::with_seeds(
        0x0f1fdc6efcc97fd9,
        0x942bd4a9d2ec6246,
        0xcf8d27c1af157eb4,
        0xda2d3937288cc846,
    )
    .build_hasher()
}

#[cfg(test)]
mod tests;
