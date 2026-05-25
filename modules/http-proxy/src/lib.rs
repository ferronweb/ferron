//! HTTP reverse proxy module for Ferron.
//!
//! Provides pipeline stages for:
//! - `ReverseProxyStage` — reverse proxying with load balancing, health checks, and connection pooling

#![cfg_attr(feature = "fuzz", allow(private_interfaces))]

mod config;
mod connections;
mod connpool_single;
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
mod util;
mod validator;

use std::collections::HashMap;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use dashmap::DashMap;
use ferron_observability::build_composite_sink;
use parking_lot::RwLock;

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
pub use send_net_io::SendTcpStreamPoll;
pub use send_net_io::SendTcpStreamPollDropGuard;
#[cfg(unix)]
pub use send_net_io::SendUnixStreamPoll;

/// Shared counter type for tracking active health check unhealthy events.
type ActiveUnhealthyCounters = parking_lot::Mutex<std::collections::HashMap<String, u64>>;

/// Metrics collected during a proxy request, emitted after completion.
pub struct ProxyMetrics {
    /// Backends selected during load balancing.
    pub selected_backends: Vec<types::upstream::UpstreamInner>,
    /// Backends marked as unhealthy due to passive failures (request-time).
    pub unhealthy_backends: Vec<types::upstream::UpstreamInner>,
    /// Backends whose circuit breaker was opened by request-time failures or 5xx responses.
    pub circuit_breaker_unhealthy_backends: Vec<types::upstream::UpstreamInner>,
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
}

impl Default for ProxyMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl ProxyMetrics {
    pub fn new() -> Self {
        Self {
            selected_backends: Vec::new(),
            unhealthy_backends: Vec::new(),
            circuit_breaker_unhealthy_backends: Vec::new(),
            active_unhealthy_backends: Vec::new(),
            connection_reused: false,
            tls_handshake_failures: 0,
            tls_handshake_time_secs: 0.0,
            pool_waits: 0,
            pool_wait_time_secs: 0.0,
            upstream_time_secs: 0.0,
            status_code: None,
        }
    }
}

const DEFAULT_KEEPALIVE_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
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

fn emit_proxy_failure_metric(
    ctx: &HttpContext,
    status_code: u16,
    result: &'static str,
    error_type: &str,
) {
    use ferron_observability::{Event, MetricAttributeValue, MetricEvent, MetricType, MetricValue};

    ctx.events.emit(Event::Metric(MetricEvent {
        name: "ferron.proxy.failures",
        attributes: vec![
            (
                "ferron.proxy.result",
                MetricAttributeValue::StaticStr(result),
            ),
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

/// Shared state for the reverse proxy stage, constructed once and reused
/// across all requests to preserve connection pools, health tracking,
/// and the load balancer algorithm (which must be shared for RoundRobin to work).
struct ProxyState {
    /// Connection pool manager — lazily initialized on first use so we can
    /// read the global `concurrent_conns` limit from config first.
    conn_manager: RwLock<Option<Arc<crate::connections::ConnectionManager>>>,
    /// Failed backend tracking cache (shared across all requests).
    failed_backends: Arc<crate::util::FailureCache>,
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
    algorithms: DashMap<
        Vec<usize>,
        (
            Arc<LoadBalancerAlgorithmInner>,
            Arc<RwLock<ConsistentHashRing>>,
        ),
    >,
    /// Active health check state tracking per upstream URL.
    active_health_check_state: types::health::HealthCheckStateMap,
    /// Background health check task handles, keyed by configuration pointer.
    /// Used to clean up tasks on reload.
    health_check_tasks: DashMap<Vec<usize>, tokio::task::JoinHandle<()>>,
    /// Counters for active health check unhealthy events, keyed by configuration pointer.
    active_unhealthy_counters: DashMap<Vec<usize>, Arc<ActiveUnhealthyCounters>>,
}

impl ProxyState {
    fn new() -> Self {
        Self {
            conn_manager: RwLock::new(None),
            failed_backends: Arc::new(RwLock::new(crate::util::TtlCache::new(
                DEFAULT_KEEPALIVE_IDLE_TIMEOUT,
            ))),
            circuit_breaker_state: Arc::new(DashMap::new()),
            conn_state: Arc::new(DashMap::new()),
            ewma_state: Arc::new(DashMap::new()),
            algorithms: DashMap::new(),
            active_health_check_state: Arc::new(DashMap::new()),
            health_check_tasks: DashMap::new(),
            active_unhealthy_counters: DashMap::new(),
        }
    }

    /// Get or create the connection manager using the globally configured limit.
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
                    let mut guard = counter_clone.lock();
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

        modules.push(Arc::new(ReverseProxyModule {
            sink: build_composite_sink(&registry, &config.global_config)?,
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
        // Capture the secondary Tokio runtime handle for SRV lookups
        let _handle = get_secondary_runtime_handle(runtime, self.sink.clone());
        self.sink.emit(ferron_observability::Event::Log(
            ferron_observability::LogEvent {
                level: ferron_observability::LogLevel::Info,
                message: "Reverse proxy module initialized".to_string(),
                target: LOG_TARGET,
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
                    .resolve(
                        Arc::clone(&self.state.failed_backends),
                        config.passive_check.max_fails,
                        Some(Arc::clone(&self.state.active_health_check_state)),
                    )
                    .await;
                for resolved_upstream in resolved {
                    conn_manager.set_local_limit(&resolved_upstream, limit);
                }
            }
        }

        let (algorithm, ring) = if let Some(algo) = self.state.algorithms.get(&config_key) {
            algo.clone()
        } else {
            self.state
                .algorithms
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
            Arc::clone(&self.state.failed_backends),
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
                        message: format!("Proxy error: {e}"),
                        trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
                    },
                ));
                emit_proxy_failure_metric(ctx, 502, "error", "backend_error");
                ctx.res = Some(ferron_http::HttpResponse::BuiltinError(502, None));
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
                }));
        }

        // Emit per-backend unhealthy metrics (passive failures)
        for backend in &metrics.unhealthy_backends {
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
                MetricAttributeValue::String("passive".to_string()),
            ));
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.backends.unhealthy",
                    attributes: attrs,
                    ty: MetricType::Counter,
                    value: MetricValue::U64(1),
                    unit: Some("{backend}"),
                    description: Some("Number of health check failures for a backend server."),
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
            }));

        let mut upstream_attrs = vec![];
        if let Some(backend) = metrics.selected_backends.last() {
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
                }));
        }

        // Emit pool wait time histogram
        if metrics.pool_wait_time_secs > 0.0 {
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.pool.wait_time",
                    attributes: upstream_attrs.clone(),
                    ty: MetricType::Histogram(Some(vec![
                        0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0,
                    ])),
                    value: MetricValue::F64(metrics.pool_wait_time_secs),
                    unit: Some("s"),
                    description: Some("Duration spent waiting for a pooled connection."),
                }));
        }

        // Emit upstream duration histogram
        if metrics.upstream_time_secs > 0.0 {
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.upstream.duration",
                    attributes: upstream_attrs.clone(),
                    ty: MetricType::Histogram(Some(vec![
                        0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0,
                    ])),
                    value: MetricValue::F64(metrics.upstream_time_secs),
                    unit: Some("s"),
                    description: Some("Duration of upstream request-response."),
                }));
        }

        // Emit TLS handshake duration histogram
        if metrics.tls_handshake_time_secs > 0.0 {
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.tls.handshake_time",
                    attributes: upstream_attrs.clone(),
                    ty: MetricType::Histogram(Some(vec![
                        0.001, 0.01, 0.05, 0.1, 0.5, 1.0, 2.0, 5.0,
                    ])),
                    value: MetricValue::F64(metrics.tls_handshake_time_secs),
                    unit: Some("s"),
                    description: Some("TLS handshake duration for upstream connection."),
                }));
        }

        // Emit P2C+EWMA adaptive load balancing diagnostics for the selected backend
        if let (Some(backend), LoadBalancerAlgorithmInner::P2cEwma) =
            (metrics.selected_backends.last(), &*algorithm)
        {
            let params = P2cEwmaParams::default();
            let ewma_backend_attrs = vec![(
                "ferron.proxy.backend_url",
                MetricAttributeValue::String(backend.proxy_to.clone()),
            )];

            // Backend EWMA latency gauge
            let ewma_latency = p2c_ewma::get_decayed_ewma(&self.state.ewma_state, backend, &params);
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.lb.ewma_latency",
                    attributes: ewma_backend_attrs.clone(),
                    ty: MetricType::Gauge,
                    value: MetricValue::F64(ewma_latency),
                    unit: Some("s"),
                    description: Some("Current EWMA response latency for the selected backend."),
                }));

            // Backend active connections gauge
            let active_conns = self
                .state
                .conn_state
                .get(backend)
                .map_or(0, |e| std::sync::Arc::strong_count(e.value()) - 1);
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.lb.active_connections",
                    attributes: ewma_backend_attrs.clone(),
                    ty: MetricType::Gauge,
                    value: MetricValue::U64(active_conns as u64),
                    unit: Some("{connection}"),
                    description: Some("Active tracked connections for the selected backend."),
                }));

            // Backend warm-up state gauge
            let warming_up = p2c_ewma::is_warming_up(&self.state.ewma_state, backend);
            ctx.events
                .emit(ferron_observability::Event::Metric(MetricEvent {
                    name: "ferron.proxy.lb.warmup_state",
                    attributes: ewma_backend_attrs,
                    ty: MetricType::Gauge,
                    value: MetricValue::U64(if warming_up { 1 } else { 0 }),
                    unit: Some("{state}"),
                    description: Some("Whether the selected backend is still in EWMA warm-up phase (1 = warming up, 0 = settled)."),
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
                    }));
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
