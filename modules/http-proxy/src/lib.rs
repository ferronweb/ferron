//! HTTP reverse proxy module for Ferron.
//!
//! Provides pipeline stages for:
//! - `ReverseProxyStage` — reverse proxying with load balancing, health checks, and connection pooling

#![cfg_attr(feature = "fuzz", allow(private_interfaces))]

mod config;
mod connections;
mod health_check;
mod proxy;
mod send_net_io;
mod send_request;
#[cfg(feature = "fuzz")]
pub mod types;
#[cfg(not(feature = "fuzz"))]
mod types;
#[cfg(feature = "fuzz")]
pub mod upstream;
#[cfg(not(feature = "fuzz"))]
mod upstream;
mod validator;

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, OnceLock};

use arc_swap::ArcSwap;
use dashmap::DashMap;
use ferron_http::span::HttpContextSpanExt;
use ferron_http::trace_context::current_event_trace_context;
use ferron_observability::build_composite_sink;
use ferron_observability::TraceAttributeValue;
use parking_lot::RwLock;
use rustc_hash::FxBuildHasher;

#[cfg(feature = "srv-lookup")]
use crate::types::upstream::Upstream;
use crate::types::ConnectionsTrackState;
use crate::upstream::lb::p2c_ewma::{self, P2cEwmaParams};
use crate::upstream::lb::{ConsistentHashRing, EwmaStateMap, LoadBalancerAlgorithmInner};
use crate::validator::ProxyConfigurationValidator;

// Re-export low-level send_net_io types for benchmarking and external tools
use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::loader::ModuleLoader;
use ferron_core::registry::RegistryBuilder;
use ferron_core::runtime::Runtime;
use ferron_core::Module;
use ferron_http::HttpContext;
#[cfg(unix)]
pub use send_net_io::SendUnixStreamPoll;
pub use send_net_io::{SendTcpStreamPoll, SendTcpStreamPollDropGuard};

/// Shared counter type for tracking active health check unhealthy events.
type ActiveUnhealthyCounters = parking_lot::RwLock<std::collections::HashMap<String, u64>>;

static PROXY_POOL_BUCKETS: &[f64] = &[0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0];
static PROXY_TLS_BUCKETS: &[f64] = &[0.001, 0.01, 0.05, 0.1, 0.5, 1.0, 2.0, 5.0];

/// Metrics collected during a proxy request, emitted after completion.
pub struct ProxyMetrics {
    /// Backends selected during load balancing.
    pub selected_backends: rustc_hash::FxHashSet<Arc<types::upstream::UpstreamInner>>,
    /// The final backend selected for this request.
    pub final_selected_backend: Option<Arc<types::upstream::UpstreamInner>>,
    /// Backends whose circuit breaker was opened by request-time failures or 5xx responses.
    pub circuit_breaker_unhealthy_backends: Vec<Arc<types::upstream::UpstreamInner>>,
    /// Backends marked as unhealthy due to active health check probes, with counts.
    pub active_unhealthy_backends: Vec<(String, u64)>,
    /// Whether a pooled connection was reused.
    pub connection_reused: bool,
    /// TLS handshake failure count for this request.
    pub tls_handshake_failures: u64,
    /// Total time spent in TLS handshake(s) for this request (in seconds).
    pub tls_handshake_time_secs: f64,
    /// Number of times the pool was exhausted and had to wait.
    pub pool_waits: u64,
    /// Total time spent waiting for pooled connections (in seconds).
    pub pool_wait_time_secs: f64,
    /// Total time spent waiting for the upstream request/response (in seconds).
    pub upstream_time_secs: f64,
    /// HTTP response status code from the upstream.
    pub status_code: Option<u16>,

    // -- Backend exclusion reasons --
    /// Backends excluded due to circuit breaker being open.
    pub excluded_circuit_open: Vec<Arc<types::upstream::UpstreamInner>>,
    /// Backends excluded because they were already tried in a retry loop.
    pub excluded_already_tried: Vec<Arc<types::upstream::UpstreamInner>>,
    /// Backends excluded because circuit breaker was half-open with an in-flight request.
    pub excluded_overloaded: Vec<Arc<types::upstream::UpstreamInner>>,

    // -- Retry metadata --
    /// Number of retry attempts made during this request.
    pub retry_count: u64,

    // -- Pool behavior --
    /// A pooled connection was available immediately without waiting.
    pub pool_hit: bool,
    /// No pooled connection was available; a new connection was established.
    pub pool_miss: bool,

    // -- Connection timing --
    /// Total time spent establishing new TCP/TLS connections (in seconds).
    pub connect_time_secs: f64,
    /// Time from request send to response headers received (TTFB, in seconds).
    pub ttfb_secs: f64,
}

impl Default for ProxyMetrics {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl ProxyMetrics {
    #[inline]
    pub fn new() -> Self {
        Self {
            selected_backends: rustc_hash::FxHashSet::default(),
            final_selected_backend: None,
            circuit_breaker_unhealthy_backends: Vec::new(),
            active_unhealthy_backends: Vec::new(),
            connection_reused: false,
            tls_handshake_failures: 0,
            tls_handshake_time_secs: 0.0,
            pool_waits: 0,
            pool_wait_time_secs: 0.0,
            upstream_time_secs: 0.0,
            status_code: None,
            excluded_circuit_open: Vec::new(),
            excluded_already_tried: Vec::new(),
            excluded_overloaded: Vec::new(),
            retry_count: 0,
            pool_hit: false,
            pool_miss: false,
            connect_time_secs: 0.0,
            ttfb_secs: 0.0,
        }
    }
}

const DEFAULT_CONCURRENT_CONNECTIONS: usize = 16384;
const LOG_TARGET: &str = "ferron-http-proxy";

/// Global concurrent connections limit, read from config during `register_modules`.
/// Uses `AtomicUsize` to allow updates during config reload.
static GLOBAL_CONCURRENT_CONNECTIONS: AtomicUsize =
    AtomicUsize::new(DEFAULT_CONCURRENT_CONNECTIONS);

/// Global accessor for the secondary Tokio runtime handle.
///
/// Populated during `ReverseProxyModule::start()` by spawning a task
/// that captures `tokio::runtime::Handle::current()`.
/// Used for SRV record resolution via `hickory_resolver`.
static SECONDARY_RUNTIME_HANDLE: OnceLock<(
    tokio::runtime::Handle,
    parking_lot::RwLock<Arc<ferron_observability::CompositeEventSink>>,
)> = OnceLock::new();

/// Cache of Hickory DNS resolvers keyed by DNS server IP list.
///
/// Resolvers are reused across SRV lookups that share the same DNS server
/// configuration, avoiding repeated allocation of DNS client state and
/// connection pools. The key is a sorted `Vec<IpAddr>` so that different
/// orderings of the same servers share one resolver.
#[cfg(feature = "srv-lookup")]
static RESOLVER_CACHE: OnceLock<
    parking_lot::RwLock<
        rustc_hash::FxHashMap<Vec<std::net::IpAddr>, Arc<hickory_resolver::TokioResolver>>,
    >,
> = OnceLock::new();

#[inline]
fn emit_proxy_failure_metric(
    ctx: &HttpContext,
    status_code: u16,
    error_type: &str,
    trace_context: Option<ferron_observability::EventTraceContext>,
) {
    use ferron_observability::{Event, MetricAttributeValue, MetricEvent, MetricType, MetricValue};

    ctx.events.emit(Event::Metric(MetricEvent {
        name: "ferron.proxy.failures",
        attributes: vec![
            (
                "http.response.status_code",
                MetricAttributeValue::I64(status_code as i64),
            ),
            (
                "error.type",
                MetricAttributeValue::String(error_type.to_string()),
            ),
        ],
        ty: MetricType::Counter,
        value: MetricValue::U64(1),
        unit: Some("{request}"),
        description: Some(
            "Number of reverse proxy requests that failed before a backend response was returned.",
        ),
        trace_context,
    }));
}

/// Returns the secondary runtime handle if it has been captured.
///
/// Returns `None` if `Module::start()` has not been called yet.
pub fn try_get_secondary_runtime_handle() -> Option<(
    tokio::runtime::Handle,
    Arc<ferron_observability::CompositeEventSink>,
)> {
    SECONDARY_RUNTIME_HANDLE
        .get()
        .map(|(h, s)| (h.clone(), s.read().clone()))
}

/// Returns the secondary runtime handle, initializing it if necessary.
///
/// The handle is captured during `Module::start()` by spawning a task
/// on the secondary runtime that calls `tokio::runtime::Handle::current()`.
pub fn get_secondary_runtime_handle(
    runtime: &Runtime,
    sink: Arc<ferron_observability::CompositeEventSink>,
) -> (
    tokio::runtime::Handle,
    Arc<ferron_observability::CompositeEventSink>,
) {
    let sink2 = sink.clone();
    let (h, s) = SECONDARY_RUNTIME_HANDLE.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel();
        runtime.spawn_secondary_task(async move {
            let _ = tx.send(tokio::runtime::Handle::current());
        });
        (
            rx.recv()
                .expect("failed to capture secondary runtime handle"),
            parking_lot::RwLock::new(sink2),
        )
    });
    *s.write() = sink.clone();
    (h.clone(), sink)
}

/// Returns a cached Hickory resolver for the given DNS servers, creating one
/// if it doesn't exist yet.
///
/// Returns `None` if the secondary runtime handle hasn't been captured yet
/// (i.e., `Module::start()` hasn't been called).
#[cfg(feature = "srv-lookup")]
pub(crate) fn get_or_create_resolver(
    dns_servers: &[std::net::IpAddr],
) -> Option<Arc<hickory_resolver::TokioResolver>> {
    use hickory_resolver::config::{NameServerConfig, ResolverConfig};
    use hickory_resolver::TokioResolver;

    let mut key = dns_servers.to_vec();
    key.sort();

    // Fast path: check cache with read lock
    if let Some(cache) = RESOLVER_CACHE.get() {
        if let Some(resolver) = cache.read().get(&key) {
            return Some(Arc::clone(resolver));
        }
    }

    // Slow path: build resolver and insert into cache
    let resolver_result = if !dns_servers.is_empty() {
        let mut resolver_config = ResolverConfig::default();
        for server in dns_servers {
            resolver_config.add_name_server(NameServerConfig::udp(*server));
        }
        TokioResolver::builder_with_config(
            resolver_config,
            hickory_resolver::net::runtime::TokioRuntimeProvider::new(),
        )
        .build()
    } else {
        TokioResolver::builder_tokio()
            .unwrap_or_else(|_| {
                TokioResolver::builder_with_config(
                    ResolverConfig::default(),
                    hickory_resolver::net::runtime::TokioRuntimeProvider::new(),
                )
            })
            .build()
    };

    let resolver = match resolver_result {
        Ok(r) => r,
        Err(_) => return None,
    };
    let resolver = Arc::new(resolver);

    let cache = RESOLVER_CACHE.get_or_init(Default::default);
    cache
        .write()
        .entry(key)
        .or_insert_with(|| Arc::clone(&resolver));

    Some(resolver)
}

/// Shared state for the reverse proxy stage, constructed once and reused
/// across all requests to preserve connection pools, health tracking,
/// and the load balancer algorithm (which must be shared for RoundRobin to work).
struct ProxyState {
    /// Connection pool manager — lazily initialized on first use so we can
    /// read the global `concurrent_conns` limit from config first.
    conn_manager: RwLock<Option<Arc<crate::connections::ConnectionManager>>>,
    /// Circuit breaker state tracking per upstream.
    circuit_breaker_state: types::circuit::CircuitBreakerStateMap,
    /// Connection tracking state for LeastConnections/TwoRandomChoices.
    conn_state: ConnectionsTrackState,
    /// Per-backend EWMA latency state for P2C+EWMA load balancing.
    ewma_state: EwmaStateMap,
    /// Load balancing algorithms cached per resolved configuration,
    /// along with consistent hash ring state.
    /// Round-robin counters must remain shared for a given config key.
    #[allow(clippy::type_complexity)]
    algorithms: ArcSwap<
        DashMap<
            Vec<usize>,
            (
                Arc<LoadBalancerAlgorithmInner>,
                Arc<RwLock<ConsistentHashRing>>,
            ),
            FxBuildHasher,
        >,
    >,
    /// Active health check state tracking per upstream URL.
    active_health_check_state: types::health::HealthCheckStateMap,
    /// Background health check task handles, keyed by configuration pointer.
    /// Used to clean up tasks on reload.
    health_check_tasks: DashMap<Vec<usize>, tokio::task::JoinHandle<()>, FxBuildHasher>,
    /// Counters for active health check unhealthy events, keyed by configuration pointer.
    active_unhealthy_counters: DashMap<Vec<usize>, Arc<ActiveUnhealthyCounters>, FxBuildHasher>,
}

impl ProxyState {
    fn new() -> Self {
        Self {
            conn_manager: RwLock::new(None),
            circuit_breaker_state: Arc::new(DashMap::with_hasher(FxBuildHasher)),
            conn_state: Arc::new(DashMap::with_hasher(FxBuildHasher)),
            ewma_state: Arc::new(DashMap::with_hasher(FxBuildHasher)),
            algorithms: ArcSwap::from_pointee(DashMap::with_hasher(FxBuildHasher)),
            active_health_check_state: Arc::new(DashMap::with_hasher(FxBuildHasher)),
            health_check_tasks: DashMap::with_hasher(FxBuildHasher),
            active_unhealthy_counters: DashMap::with_hasher(FxBuildHasher),
        }
    }

    /// Get or create the connection manager using the globally configured limit.
    #[inline]
    fn get_conn_manager(&self) -> Arc<crate::connections::ConnectionManager> {
        let guard = self.conn_manager.read();
        if let Some(cm) = &*guard {
            return Arc::clone(cm);
        }
        drop(guard);

        let mut guard = self.conn_manager.write();
        if let Some(cm) = &*guard {
            return Arc::clone(cm);
        }

        let limit = GLOBAL_CONCURRENT_CONNECTIONS.load(std::sync::atomic::Ordering::Relaxed);
        let cm = Arc::new(crate::connections::ConnectionManager::with_global_limit(
            limit,
        ));
        *guard = Some(Arc::clone(&cm));
        cm
    }

    /// Spawn health check task for the given config (idempotent).
    ///
    /// If a task is already running for this config, does nothing.
    #[inline]
    fn ensure_health_check_task(&self, config_keys: &[usize], upstreams: &[Upstream]) {
        // Check if task already exists
        if self.health_check_tasks.contains_key(config_keys) {
            return;
        }

        // Check if any upstream has health checks enabled
        let has_health_checks = upstreams.iter().any(|u| match u {
            Upstream::Static(cfg) => cfg.health_check_config.enabled,
            #[cfg(feature = "srv-lookup")]
            Upstream::Srv(cfg) => cfg.health_check_config.enabled,
        });

        if !has_health_checks {
            return;
        }

        // Get the secondary runtime handle for spawning the health check task
        let (runtime_handle, event_sink) = match try_get_secondary_runtime_handle() {
            Some(h) => h,
            None => {
                ferron_core::log_warn!(
                    "Health check task not spawned — secondary runtime not yet available"
                );
                return;
            }
        };

        // Spawn the health check task with a callback to update the shared counter
        if let dashmap::Entry::Vacant(e) = self.health_check_tasks.entry(config_keys.to_vec()) {
            let counter: Arc<ActiveUnhealthyCounters> =
                match self.active_unhealthy_counters.entry(config_keys.to_vec()) {
                    dashmap::Entry::Occupied(e) => e.get().clone(),
                    dashmap::Entry::Vacant(e) => {
                        let counter = Arc::new(ActiveUnhealthyCounters::new(HashMap::new()));
                        e.insert(counter.clone());
                        counter
                    }
                };
            let counter_clone = Arc::clone(&counter);
            let task = health_check::spawn_health_check_task(
                upstreams.to_vec(),
                Arc::clone(&self.active_health_check_state),
                Some(Arc::new(move |url: &str, _is_active: bool| {
                    let mut guard = counter_clone.write();
                    *guard.entry(url.to_string()).or_insert(0) += 1;
                })),
                &runtime_handle,
                event_sink,
            );

            e.insert(task);
        }
    }
}

/// Module loader for the HTTP reverse proxy module.
#[derive(Default)]
pub struct ReverseProxyModuleLoader {
    /// Shared proxy state, set during `register_stages` and used in `register_modules`.
    state: Option<Arc<ProxyState>>,
}

impl ModuleLoader for ReverseProxyModuleLoader {
    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn ConfigurationValidator>>,
    ) {
        registry.push(Box::new(ProxyConfigurationValidator));
    }

    fn register_per_protocol_configuration_validators(
        &mut self,
        registry: &mut std::collections::HashMap<
            &'static str,
            Vec<Box<dyn ferron_core::config::validator::ConfigurationValidator>>,
        >,
    ) {
        registry
            .entry("http")
            .or_default()
            .push(Box::new(ProxyConfigurationValidator));
    }

    fn register_stages(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        let state = Arc::new(ProxyState::new());
        self.state = Some(Arc::clone(&state));
        registry.with_stage::<HttpContext, _>(move || {
            Arc::new(ReverseProxyStage {
                state: Arc::clone(&state),
            })
        })
    }

    fn register_modules(
        &mut self,
        registry: Arc<ferron_core::registry::Registry>,
        modules: &mut Vec<Arc<dyn Module>>,
        config: Arc<ferron_core::config::ServerConfiguration>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let _ = self.state.as_ref().map(|s| s.get_conn_manager());

        // Read global concurrent connections limit if configured
        if let Some(val) = config
            .global_config
            .directives
            .get("proxy_concurrent_conns")
            .and_then(|entries| entries.first())
            .and_then(|e| e.args.first())
            .and_then(|v: &ferron_core::config::ServerConfigurationValue| v.as_number())
        {
            if val > 0 {
                let new_limit = val as usize;
                let old_limit =
                    GLOBAL_CONCURRENT_CONNECTIONS.load(std::sync::atomic::Ordering::Relaxed);
                GLOBAL_CONCURRENT_CONNECTIONS
                    .store(new_limit, std::sync::atomic::Ordering::Relaxed);

                // If limit changed and conn_manager already exists, update it in place
                if old_limit != new_limit {
                    if let Some(ref state) = self.state {
                        let cm = state.get_conn_manager();
                        cm.update_global_limit(new_limit);
                    }
                }
            }
        }

        // Clear mTLS cache
        self::config::MTLS_FILE_CACHE.clear();

        // Prevent load balancing state memory leaks
        if let Some(ref state) = self.state {
            state.algorithms.swap(Default::default());
        }

        modules.push(Arc::new(ReverseProxyModule {
            sink: build_composite_sink(&registry, &config.global_config, None)?,
        }));
        Ok(())
    }
}

/// The reverse proxy module.
///
/// Responsible for:
/// - Capturing the secondary Tokio runtime handle (for SRV resolution)
struct ReverseProxyModule {
    sink: Arc<ferron_observability::CompositeEventSink>,
}

impl Module for ReverseProxyModule {
    fn name(&self) -> &str {
        "reverse-proxy"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn start(&self, runtime: &mut Runtime) -> Result<(), Box<dyn std::error::Error>> {
        // Capture the secondary Tokio runtime handle for SRV lookups and pool gauge emission
        let (secondary_handle, pool_sink) =
            get_secondary_runtime_handle(runtime, self.sink.clone());

        // Spawn periodic pool depth gauge emission on the secondary runtime
        secondary_handle.spawn(async move {
            use ferron_observability::{
                Event, MetricAttributeValue, MetricEvent, MetricType, MetricValue,
            };

            let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
            loop {
                interval.tick().await;
                let snapshot = crate::connections::POOL_STATS.snapshot();
                for ((thread_id, upstream), (idle, outstanding)) in snapshot {
                    let mut attrs = Vec::with_capacity(3);
                    attrs.push((
                        "ferron.proxy.backend_url",
                        MetricAttributeValue::String(upstream.proxy_to.clone()),
                    ));
                    if let Some(ref unix_path) = upstream.proxy_unix {
                        attrs.push((
                            "ferron.proxy.backend_unix_path",
                            MetricAttributeValue::String(unix_path.clone()),
                        ));
                    }
                    attrs.push((
                        "worker",
                        MetricAttributeValue::String(format!("{:?}", thread_id)),
                    ));
                    pool_sink.emit(Event::Metric(MetricEvent {
                        name: "ferron.proxy.pool.idle",
                        attributes: attrs.clone(),
                        ty: MetricType::Gauge,
                        value: MetricValue::U64(idle as u64),
                        unit: Some("{connection}"),
                        description: Some("Current number of idle connections in the pool."),
                        trace_context: None,
                    }));
                    pool_sink.emit(Event::Metric(MetricEvent {
                        name: "ferron.proxy.pool.outstanding",
                        attributes: attrs,
                        ty: MetricType::Gauge,
                        value: MetricValue::U64(outstanding as u64),
                        unit: Some("{connection}"),
                        description: Some(
                            "Current number of outstanding (in-use) connections in the pool.",
                        ),
                        trace_context: None,
                    }));
                }

                // Emit DNS cache hit/miss counters
                let hits = crate::types::dns_cache::DNS_CACHE_HITS
                    .swap(0, std::sync::atomic::Ordering::Relaxed);
                let misses = crate::types::dns_cache::DNS_CACHE_MISSES
                    .swap(0, std::sync::atomic::Ordering::Relaxed);
                if hits > 0 {
                    pool_sink.emit(Event::Metric(MetricEvent {
                        name: "ferron.proxy.dns.cache_hit",
                        attributes: Vec::new(),
                        ty: MetricType::Counter,
                        value: MetricValue::U64(hits),
                        unit: Some("{request}"),
                        description: Some("DNS result cache hits."),
                        trace_context: None,
                    }));
                }
                if misses > 0 {
                    pool_sink.emit(Event::Metric(MetricEvent {
                        name: "ferron.proxy.dns.cache_miss",
                        attributes: Vec::new(),
                        ty: MetricType::Counter,
                        value: MetricValue::U64(misses),
                        unit: Some("{request}"),
                        description: Some("DNS result cache misses."),
                        trace_context: None,
                    }));
                }
            }
        });

        // Spawn periodic DNS result cache cleanup on the secondary runtime
        secondary_handle.spawn(async {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                interval.tick().await;
                crate::types::dns_cache::cleanup_expired();
            }
        });

        self.sink.emit(ferron_observability::Event::Log(
            ferron_observability::LogEvent {
                level: ferron_observability::LogLevel::Debug,
                message: "Reverse proxy module initialized".to_string(),
                summary: "Reverse proxy module initialized".into(),
                target: LOG_TARGET,
                attributes: Vec::new(),
                trace_context: None,
            },
        ));
        Ok(())
    }
}

struct ReverseProxyStage {
    state: Arc<ProxyState>,
}

#[async_trait::async_trait(?Send)]
impl ferron_core::pipeline::Stage<HttpContext> for ReverseProxyStage {
    fn name(&self) -> &str {
        "reverse_proxy"
    }

    fn is_applicable(
        &self,
        config: Option<&ferron_core::config::ServerConfigurationBlock>,
    ) -> bool {
        config.is_some_and(|c| c.has_directive("proxy"))
    }

    async fn run(
        &self,
        ctx: &mut HttpContext,
    ) -> Result<bool, ferron_core::pipeline::PipelineError> {
        let entries = ctx.configuration.get_entries("proxy", true);
        if entries.is_empty() {
            return Ok(true);
        }

        // Use the layer Arc pointer identities as a cache key.
        // When config is reloaded, new Arc pointers are created.
        let config_key = ctx
            .configuration
            .layers
            .iter()
            .filter_map(|arc| {
                if arc.has_directive("proxy") {
                    Some(Arc::as_ptr(arc) as usize)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        let config = match config::parse_proxy_config(ctx) {
            Ok(Some(cfg)) => Arc::new(cfg),
            Ok(None) => return Ok(true),
            Err(e) => {
                ctx.events.emit(ferron_observability::Event::Log(
                    ferron_observability::LogEvent {
                        target: "ferron-proxy",
                        level: ferron_observability::LogLevel::Error,
                        message: format!("Proxy config error: {e}"),
                        summary: "Reverse proxy config error".into(),
                        attributes: vec![(
                            "error.message",
                            ferron_observability::LogAttributeValue::String(e.to_string()),
                        )],
                        trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
                    },
                ));
                return Ok(true);
            }
        };

        // Spawn health check task for this config if needed
        self.state
            .ensure_health_check_task(&config_key, &config.upstreams);

        // Set or update per-upstream local limits.
        let conn_manager = self.state.get_conn_manager();
        for uc in &config.upstreams {
            let limit = match uc {
                Upstream::Static(s) => s.limit,
                #[cfg(feature = "srv-lookup")]
                Upstream::Srv(s) => s.limit,
            };
            if let Some(limit) = limit {
                let resolved = uc
                    .resolve(Some(Arc::clone(&self.state.active_health_check_state)))
                    .await;
                for resolved_upstream in resolved {
                    conn_manager.set_local_limit(resolved_upstream, limit);
                }
            }
        }

        let (algorithm, ring) = if let Some(algo) = self.state.algorithms.load().get(&config_key) {
            algo.clone()
        } else {
            self.state
                .algorithms
                .load()
                .entry(config_key.clone())
                .or_insert_with(|| {
                    (
                        Arc::new(config.algorithm.into()),
                        // Blank upstream list for now
                        Arc::new(RwLock::new(ConsistentHashRing::new(&[]))),
                    )
                })
                .clone()
        };

        // Get the active unhealthy counter for this config
        let active_unhealthy_counter = {
            self.state
                .active_unhealthy_counters
                .get(&config_key)
                .as_deref()
                .cloned()
        };

        let result = proxy::execute_proxy(
            ctx,
            &config,
            &conn_manager,
            Arc::clone(&self.state.circuit_breaker_state),
            &algorithm,
            &ring,
            Some(&self.state.conn_state),
            Some(&self.state.ewma_state),
            Some(&self.state.active_health_check_state),
            active_unhealthy_counter.as_deref(),
        )
        .await;

        let (response, metrics) = match result {
            Ok((resp, m)) => (resp, m),
            Err(e) => {
                ctx.events.emit(ferron_observability::Event::Log(
                    ferron_observability::LogEvent {
                        target: "ferron-proxy",
                        level: ferron_observability::LogLevel::Error,
                        message: format!("Proxy error: {}", e),
                        summary: e.summary().into(),
                        attributes: vec![
                            (
                                "error.type",
                                ferron_observability::LogAttributeValue::String(
                                    e.error_type().to_string(),
                                ),
                            ),
                            (
                                "error.message",
                                ferron_observability::LogAttributeValue::String(e.to_string()),
                            ),
                        ],
                        trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
                    },
                ));
                let status_code = e.http_status_hint().map_or(502, |sh| sh.as_u16());
                emit_proxy_failure_metric(
                    ctx,
                    status_code,
                    e.error_type(),
                    current_event_trace_context(ctx),
                );
                ctx.res = Some(ferron_http::HttpResponse::BuiltinError(status_code, None));
                ctx.get_span_attributes().insert(
                    "http.response.status_code",
                    TraceAttributeValue::I64(status_code as i64),
                );
                ctx.get_span_attributes().insert(
                    "error.type",
                    TraceAttributeValue::String(e.error_type().to_string()),
                );
                return Ok(false);
            }
        };

        ctx.res = Some(response);

        // Emit per-backend selected metrics
        use ferron_observability::{MetricAttributeValue, MetricEvent, MetricType, MetricValue};
        for backend in &metrics.selected_backends {
            let mut attrs = Vec::with_capacity(2);
            attrs.push((
                "ferron.proxy.backend_url",
                MetricAttributeValue::String(backend.proxy_to.clone()),
            ));
            if let Some(ref unix_path) = backend.proxy_unix {
                attrs.push((
                    "ferron.proxy.backend_unix_path",
                    MetricAttributeValue::String(unix_path.clone()),
                ));
            }
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.backends.selected",
                    attributes: attrs,
                    ty: MetricType::Counter,
                    value: MetricValue::U64(1),
                    unit: Some("{backend}"),
                    description: Some("Number of times a backend server was selected."),
                    trace_context: current_event_trace_context(ctx),
                }));
        }

        // Emit per-backend circuit breaker unhealthy metrics
        for backend in &metrics.circuit_breaker_unhealthy_backends {
            let mut attrs = Vec::with_capacity(3);
            attrs.push((
                "ferron.proxy.backend_url",
                MetricAttributeValue::String(backend.proxy_to.clone()),
            ));
            if let Some(ref unix_path) = backend.proxy_unix {
                attrs.push((
                    "ferron.proxy.backend_unix_path",
                    MetricAttributeValue::String(unix_path.clone()),
                ));
            }
            attrs.push((
                "ferron.proxy.health_check_type",
                MetricAttributeValue::String("circuit_breaker".to_string()),
            ));
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.backends.unhealthy",
                    attributes: attrs,
                    ty: MetricType::Counter,
                    value: MetricValue::U64(1),
                    unit: Some("{backend}"),
                    description: Some("Number of health check failures for a backend server."),
                    trace_context: current_event_trace_context(ctx),
                }));
        }

        // Emit per-backend active health check unhealthy metrics
        for (backend_url, count) in &metrics.active_unhealthy_backends {
            let attrs = vec![
                (
                    "ferron.proxy.backend_url",
                    MetricAttributeValue::String(backend_url.clone()),
                ),
                (
                    "ferron.proxy.health_check_type",
                    MetricAttributeValue::String("active".to_string()),
                ),
            ];
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.backends.unhealthy",
                    attributes: attrs,
                    ty: MetricType::Counter,
                    value: MetricValue::U64(*count),
                    unit: Some("{backend}"),
                    description: Some("Number of health check failures for a backend server."),
                    trace_context: current_event_trace_context(ctx),
                }));
        }

        // Emit request counter with connection reuse flag and status code
        let mut request_attrs = Vec::with_capacity(3);
        request_attrs.push((
            "ferron.proxy.connection_reused",
            MetricAttributeValue::Bool(metrics.connection_reused),
        ));
        if let Some(status) = metrics.status_code {
            request_attrs.push((
                "http.response.status_code",
                MetricAttributeValue::I64(status as i64),
            ));
        }
        request_attrs.push((
            "ferron.proxy.status_code",
            MetricAttributeValue::I64(metrics.status_code.map(|s| s as i64).unwrap_or(0)),
        ));
        ctx.events
            .emit(ferron_observability::Event::Metric(MetricEvent {
                name: "ferron.proxy.requests",
                attributes: request_attrs,
                ty: MetricType::Counter,
                value: MetricValue::U64(1),
                unit: Some("{request}"),
                description: Some("Number of reverse proxy requests."),
                trace_context: current_event_trace_context(ctx),
            }));

        let mut upstream_attrs = vec![];
        if let Some(backend) = metrics.final_selected_backend.as_ref() {
            upstream_attrs.push((
                "ferron.proxy.backend_url",
                MetricAttributeValue::String(backend.proxy_to.clone()),
            ));
            if let Some(ref unix_path) = backend.proxy_unix {
                upstream_attrs.push((
                    "ferron.proxy.backend_unix_path",
                    MetricAttributeValue::String(unix_path.clone()),
                ));
            }
        }

        // Emit TLS handshake failures counter
        if metrics.tls_handshake_failures > 0 {
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.tls_handshake_failures",
                    attributes: upstream_attrs.clone(),
                    ty: MetricType::Counter,
                    value: MetricValue::U64(metrics.tls_handshake_failures),
                    unit: Some("{handshake}"),
                    description: Some("TLS handshake failures with upstream backends."),
                    trace_context: current_event_trace_context(ctx),
                }));
        }

        // Emit pool waits counter
        if metrics.pool_waits > 0 {
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.pool.waits",
                    attributes: upstream_attrs.clone(),
                    ty: MetricType::Counter,
                    value: MetricValue::U64(metrics.pool_waits),
                    unit: Some("{wait}"),
                    description: Some(
                        "Times the connection pool was exhausted and a request had to wait.",
                    ),
                    trace_context: current_event_trace_context(ctx),
                }));
        }

        // Emit pool wait time histogram
        if metrics.pool_wait_time_secs > 0.0 {
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.pool.wait_time",
                    attributes: upstream_attrs.clone(),
                    ty: MetricType::Histogram(Some(Cow::Borrowed(PROXY_POOL_BUCKETS))),
                    value: MetricValue::F64(metrics.pool_wait_time_secs),
                    unit: Some("s"),
                    description: Some("Duration spent waiting for a pooled connection."),
                    trace_context: current_event_trace_context(ctx),
                }));
        }

        // Emit upstream duration histogram
        if metrics.upstream_time_secs > 0.0 {
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.upstream.duration",
                    attributes: upstream_attrs.clone(),
                    ty: MetricType::Histogram(Some(Cow::Borrowed(PROXY_POOL_BUCKETS))),
                    value: MetricValue::F64(metrics.upstream_time_secs),
                    unit: Some("s"),
                    description: Some("Duration of upstream request-response."),
                    trace_context: current_event_trace_context(ctx),
                }));
        }

        // Emit TLS handshake duration histogram
        if metrics.tls_handshake_time_secs > 0.0 {
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.tls.handshake_time",
                    attributes: upstream_attrs.clone(),
                    ty: MetricType::Histogram(Some(Cow::Borrowed(PROXY_TLS_BUCKETS))),
                    value: MetricValue::F64(metrics.tls_handshake_time_secs),
                    unit: Some("s"),
                    description: Some("TLS handshake duration for upstream connection."),
                    trace_context: current_event_trace_context(ctx),
                }));
        }

        // --- Backend exclusion reasons ---
        fn emit_backend_excluded(
            events: &ferron_observability::CompositeEventSink,
            backend: &Arc<types::upstream::UpstreamInner>,
            reason: &'static str,
            trace_context: Option<ferron_observability::EventTraceContext>,
        ) {
            let mut attrs = Vec::with_capacity(3);
            attrs.push((
                "ferron.proxy.backend_url",
                MetricAttributeValue::String(backend.proxy_to.clone()),
            ));
            if let Some(ref unix_path) = backend.proxy_unix {
                attrs.push((
                    "ferron.proxy.backend_unix_path",
                    MetricAttributeValue::String(unix_path.clone()),
                ));
            }
            attrs.push((
                "ferron.proxy.reason",
                MetricAttributeValue::StaticStr(reason),
            ));
            events.emit(ferron_observability::Event::Metric(MetricEvent {
                name: "ferron.proxy.backend.excluded",
                attributes: attrs,
                ty: MetricType::Counter,
                value: MetricValue::U64(1),
                unit: Some("{backend}"),
                description: Some("Backend excluded from selection due to health, circuit breaker, or retry state."),
                trace_context,
            }));
        }
        for backend in &metrics.excluded_circuit_open {
            emit_backend_excluded(
                &ctx.events,
                backend,
                "circuit_open",
                current_event_trace_context(ctx),
            );
        }
        for backend in &metrics.excluded_already_tried {
            emit_backend_excluded(
                &ctx.events,
                backend,
                "already_tried",
                current_event_trace_context(ctx),
            );
        }
        for backend in &metrics.excluded_overloaded {
            emit_backend_excluded(
                &ctx.events,
                backend,
                "overloaded",
                current_event_trace_context(ctx),
            );
        }

        // --- Retry metrics ---
        if metrics.retry_count > 0 {
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.retry.count",
                    attributes: upstream_attrs.clone(),
                    ty: MetricType::Counter,
                    value: MetricValue::U64(metrics.retry_count),
                    unit: Some("{attempt}"),
                    description: Some("Number of retry attempts during backend selection."),

                    trace_context: current_event_trace_context(ctx),
                }));
            let mut final_attrs = upstream_attrs.clone();
            final_attrs.push(("ferron.proxy.retry.final", MetricAttributeValue::Bool(true)));
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                name: "ferron.proxy.retry.final",
                attributes: final_attrs,
                ty: MetricType::Gauge,
                value: MetricValue::U64(1),
                unit: Some("{request}"),
                description: Some(
                    "Indicates the request succeeded after a retry (1) or required no retries (0).",
                ),
                trace_context: current_event_trace_context(ctx),
            }));
        }

        // --- Pool hit / miss ---
        if metrics.pool_hit {
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.pool.hit",
                    attributes: upstream_attrs.clone(),
                    ty: MetricType::Counter,
                    value: MetricValue::U64(1),
                    unit: Some("{request}"),
                    description: Some("A pooled connection was available immediately."),
                    trace_context: current_event_trace_context(ctx),
                }));
        }
        if metrics.pool_miss {
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.pool.miss",
                    attributes: upstream_attrs.clone(),
                    ty: MetricType::Counter,
                    value: MetricValue::U64(1),
                    unit: Some("{request}"),
                    description: Some(
                        "No pooled connection was available; a new connection was established.",
                    ),
                    trace_context: current_event_trace_context(ctx),
                }));
        }

        // --- Connection latency histograms ---
        if metrics.connect_time_secs > 0.0 {
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.connect.latency",
                    attributes: upstream_attrs.clone(),
                    ty: MetricType::Histogram(Some(Cow::Borrowed(PROXY_POOL_BUCKETS))),
                    value: MetricValue::F64(metrics.connect_time_secs),
                    unit: Some("s"),
                    description: Some(
                        "Duration of TCP/TLS connection establishment to the upstream.",
                    ),
                    trace_context: current_event_trace_context(ctx),
                }));
        }
        if metrics.ttfb_secs > 0.0 {
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.ttfb",
                    attributes: upstream_attrs.clone(),
                    ty: MetricType::Histogram(Some(Cow::Borrowed(PROXY_POOL_BUCKETS))),
                    value: MetricValue::F64(metrics.ttfb_secs),
                    unit: Some("s"),
                    description: Some(
                        "Time from request send to first byte of response headers received.",
                    ),
                    trace_context: current_event_trace_context(ctx),
                }));
        }

        if let Some(backend) = metrics.final_selected_backend.as_ref() {
            // Backend active connections gauge
            let active_conns = self
                .state
                .conn_state
                .get(backend)
                .map_or(0, |e| std::sync::Arc::strong_count(e.value()) - 1);
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.lb.active_connections",
                    attributes: upstream_attrs.clone(),
                    ty: MetricType::Gauge,
                    value: MetricValue::U64(active_conns as u64),
                    unit: Some("{connection}"),
                    description: Some("Active tracked connections for the selected backend."),
                    trace_context: current_event_trace_context(ctx),
                }));

            // Emit P2C+EWMA adaptive load balancing diagnostics for the selected backend
            if matches!(&*algorithm, LoadBalancerAlgorithmInner::P2cEwma) {
                let params = P2cEwmaParams::default();

                // Backend EWMA latency gauge
                let ewma_latency =
                    p2c_ewma::get_decayed_ewma(&self.state.ewma_state, backend, &params);
                ctx.events
                    .emit(ferron_observability::Event::Metric(MetricEvent {
                        name: "ferron.proxy.lb.ewma_latency",
                        attributes: upstream_attrs.clone(),
                        ty: MetricType::Gauge,
                        value: MetricValue::F64(ewma_latency),
                        unit: Some("s"),
                        description: Some(
                            "Current EWMA response latency for the selected backend.",
                        ),
                        trace_context: current_event_trace_context(ctx),
                    }));

                // Backend warm-up state gauge
                let warming_up = p2c_ewma::is_warming_up(&self.state.ewma_state, backend);
                ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.lb.warmup_state",
                    attributes: upstream_attrs.clone(),
                    ty: MetricType::Gauge,
                    value: MetricValue::U64(if warming_up { 1 } else { 0 }),
                    unit: Some("{state}"),
                    description: Some("Whether the selected backend is still in EWMA warm-up phase (1 = warming up, 0 = settled)."),
                    trace_context: current_event_trace_context(ctx),
                }));

                // Emit routing decision counter with reason
                if ewma_latency > 0.0 {
                    let score = p2c_ewma::compute_score(ewma_latency, active_conns, &params);
                    let mut sel_attrs = upstream_attrs.clone();
                    sel_attrs.push((
                        "ferron.proxy.lb.reason",
                        MetricAttributeValue::String("p2c_ewma".to_string()),
                    ));
                    sel_attrs.push(("ferron.proxy.lb.score", MetricAttributeValue::F64(score)));
                    ctx.events
                        .emit(ferron_observability::Event::Metric(MetricEvent {
                            name: "ferron.proxy.lb.selections",
                            attributes: sel_attrs,
                            ty: MetricType::Counter,
                            value: MetricValue::U64(1),
                            unit: Some("{selection}"),
                            description: Some("P2C+EWMA backend selection with combined score."),

                            trace_context: current_event_trace_context(ctx),
                        }));
                }
            }
        }

        let sa = ctx.get_span_attributes();
        if let Some(status) = metrics.status_code {
            sa.insert(
                "http.response.status_code",
                TraceAttributeValue::I64(status as i64),
            );
        }
        sa.insert(
            "ferron.proxy.connection_reused",
            TraceAttributeValue::Bool(metrics.connection_reused),
        );
        sa.insert(
            "ferron.proxy.retry_count",
            TraceAttributeValue::I64(metrics.retry_count as i64),
        );
        if let Some(backend) = metrics.final_selected_backend.as_ref() {
            sa.insert(
                "ferron.proxy.backend_url",
                TraceAttributeValue::String(backend.proxy_to.clone()),
            );
            if let Some(ref unix_path) = backend.proxy_unix {
                sa.insert(
                    "ferron.proxy.backend_unix_path",
                    TraceAttributeValue::String(unix_path.clone()),
                );
            }
        }

        Ok(false)
    }

    async fn run_inverse(
        &self,
        _ctx: &mut HttpContext,
    ) -> Result<(), ferron_core::pipeline::PipelineError> {
        Ok(())
    }
}
