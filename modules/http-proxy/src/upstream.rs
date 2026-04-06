//! Upstream resolution and load balancing logic.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;

use crate::util::TtlCache;

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
    use hickory_resolver::config::{NameServerConfig, Protocol, ResolverConfig, ResolverOpts};
    use hickory_resolver::TokioAsyncResolver;

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
                    });
                }
                TokioAsyncResolver::tokio(resolver_config, ResolverOpts::default())
            } else {
                TokioAsyncResolver::tokio_from_system_conf().unwrap_or_else(|_| {
                    TokioAsyncResolver::tokio(ResolverConfig::default(), ResolverOpts::default())
                })
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
            let failed = failed_backends.read().await;
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
async fn select_backend_index(
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
                let connection_track_read = conn_state.read().await;
                let connection_count = if let Some(tracker) = connection_track_read.get(upstream) {
                    Arc::strong_count(tracker) - 1
                } else {
                    drop(connection_track_read);
                    conn_state
                        .write()
                        .await
                        .insert(upstream.clone(), Arc::new(()));
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
                let read = conn_state.read().await;
                if read.get(&backends[0]).is_none() {
                    drop(read);
                    conn_state
                        .write()
                        .await
                        .insert(backends[0].clone(), Arc::new(()));
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
                let read = conn_state.read().await;
                if let Some(t) = read.get(&backends[idx1]) {
                    (Arc::strong_count(t) - 1, false)
                } else {
                    drop(read);
                    conn_state
                        .write()
                        .await
                        .insert(backends[idx1].clone(), Arc::new(()));
                    (0, true)
                }
            };

            // Get count for second backend
            let count2 = {
                let read = conn_state.read().await;
                if let Some(t) = read.get(&backends[idx2]) {
                    Arc::strong_count(t) - 1
                } else {
                    drop(read);
                    conn_state
                        .write()
                        .await
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
/// Filters out unhealthy backends when health checking is enabled.
pub async fn determine_proxy_to(
    upstreams: &[UpstreamInner],
    failed_backends: &RwLock<TtlCache<UpstreamInner, u64>>,
    health_check_enabled: bool,
    health_check_max_fails: u64,
    algorithm: &LoadBalancerAlgorithmInner,
    conn_state: Option<&ConnectionsTrackState>,
) -> Option<SelectedBackend> {
    if upstreams.is_empty() {
        return None;
    }

    // Build a mutable copy of healthy backends for the selection loop
    let mut healthy: Vec<UpstreamInner> = if health_check_enabled {
        let failed = failed_backends.read().await;
        upstreams
            .iter()
            .filter(|u| {
                failed
                    .get(*u)
                    .is_none_or(|fails| fails <= health_check_max_fails)
            })
            .cloned()
            .collect()
    } else {
        upstreams.to_vec()
    };

    if healthy.is_empty() {
        return None;
    }

    if healthy.len() == 1 {
        // Single backend — initialize tracker if needed
        let tracker = initialize_tracker(conn_state, &healthy[0]).await;
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
        let index = select_backend_index(algorithm, &healthy, conn_state).await;
        let upstream = healthy.remove(index);

        if health_check_enabled {
            let failed = failed_backends.read().await;
            if let Some(fails) = failed.get(&upstream) {
                if fails > health_check_max_fails {
                    continue; // Skip unhealthy, try next
                }
            }
        }

        // Get the tracker (already initialized by select_backend_index)
        let tracker = get_tracker(conn_state, &upstream).await;
        return Some(SelectedBackend { upstream, tracker });
    }
}

/// Get or create the connection tracker for an upstream.
async fn initialize_tracker(
    conn_state: Option<&ConnectionsTrackState>,
    upstream: &UpstreamInner,
) -> Option<Arc<()>> {
    let conn_state = conn_state?;
    let read = conn_state.read().await;
    if read.get(upstream).is_some() {
        return None; // Tracker already exists, caller will clone it
    }
    drop(read);
    conn_state
        .write()
        .await
        .insert(upstream.clone(), Arc::new(()));
    None
}

/// Clone an existing connection tracker for an upstream.
async fn get_tracker(
    conn_state: Option<&ConnectionsTrackState>,
    upstream: &UpstreamInner,
) -> Option<Arc<()>> {
    let conn_state = conn_state?;
    conn_state.read().await.get(upstream).map(Arc::clone)
}

/// Mark a backend as failed.
pub async fn mark_backend_failure(
    failed_backends: Arc<tokio::sync::RwLock<TtlCache<UpstreamInner, u64>>>,
    health_check_enabled: bool,
    upstream: &UpstreamInner,
    metrics: &mut crate::ProxyMetrics,
) {
    if !health_check_enabled {
        return;
    }
    metrics.unhealthy_backends.push(upstream.clone());
    let mut failed = failed_backends.write().await;
    let current = failed.get(upstream).unwrap_or(0);
    failed.insert(upstream.clone(), current + 1);
}
