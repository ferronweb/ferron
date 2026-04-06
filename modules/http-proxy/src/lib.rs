//! HTTP reverse proxy module for Ferron.

mod config;
mod connections;
mod proxy;
mod send_net_io;
mod send_request;
mod upstream;
mod util;

use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::loader::ModuleLoader;
use ferron_core::registry::RegistryBuilder;
use ferron_core::runtime::Runtime;
use ferron_core::Module;
use ferron_http::HttpContext;
use tokio::sync::RwLock;

pub use config::ProxyConfigurationValidator;

/// Metrics collected during a proxy request, emitted after completion.
pub struct ProxyMetrics {
    /// Backends selected during load balancing.
    pub selected_backends: Vec<upstream::UpstreamInner>,
    /// Backends marked as unhealthy due to failures.
    pub unhealthy_backends: Vec<upstream::UpstreamInner>,
    /// Whether a pooled connection was reused.
    pub connection_reused: bool,
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
            connection_reused: false,
        }
    }
}

const DEFAULT_KEEPALIVE_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_CONCURRENT_CONNECTIONS: usize = 16384;

/// Global concurrent connections limit, read from config during `register_modules`.
static GLOBAL_CONCURRENT_CONNECTIONS: OnceLock<usize> = OnceLock::new();

/// Global accessor for the secondary Tokio runtime handle.
///
/// Populated during `ReverseProxyModule::start()` by spawning a task
/// that captures `tokio::runtime::Handle::current()`.
/// Used for SRV record resolution via `hickory_resolver`.
static SECONDARY_RUNTIME_HANDLE: OnceLock<tokio::runtime::Handle> = OnceLock::new();

/// Returns the secondary runtime handle if it has been captured.
///
/// Returns `None` if `Module::start()` has not been called yet.
pub fn try_get_secondary_runtime_handle() -> Option<tokio::runtime::Handle> {
    SECONDARY_RUNTIME_HANDLE.get().cloned()
}

/// Returns the secondary runtime handle, initializing it if necessary.
///
/// The handle is captured during `Module::start()` by spawning a task
/// on the secondary runtime that calls `tokio::runtime::Handle::current()`.
pub fn get_secondary_runtime_handle(runtime: &Runtime) -> tokio::runtime::Handle {
    SECONDARY_RUNTIME_HANDLE
        .get_or_init(|| {
            let (tx, rx) = std::sync::mpsc::channel();
            runtime.spawn_secondary_task(async move {
                let _ = tx.send(tokio::runtime::Handle::current());
            });
            rx.recv()
                .expect("failed to capture secondary runtime handle")
        })
        .clone()
}

/// Shared state for the reverse proxy stage, constructed once and reused
/// across all requests to preserve connection pools and health tracking.
struct ProxyState {
    /// Connection pool manager — lazily initialized on first use so we can
    /// read the global `concurrent_conns` limit from config first.
    conn_manager: RwLock<Option<Arc<crate::connections::ConnectionManager>>>,
    /// Failed backend tracking cache (shared across all requests).
    failed_backends: Arc<RwLock<crate::util::TtlCache<upstream::UpstreamInner, u64>>>,
    /// Connection tracking state for LeastConnections/TwoRandomChoices.
    conn_state: upstream::ConnectionsTrackState,
}

impl ProxyState {
    fn new() -> Self {
        Self {
            conn_manager: RwLock::new(None),
            failed_backends: Arc::new(RwLock::new(crate::util::TtlCache::new(
                DEFAULT_KEEPALIVE_IDLE_TIMEOUT,
            ))),
            conn_state: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }

    /// Get or create the connection manager using the globally configured limit.
    async fn get_conn_manager(&self) -> Arc<crate::connections::ConnectionManager> {
        let guard = self.conn_manager.read().await;
        if let Some(cm) = &*guard {
            return Arc::clone(cm);
        }
        drop(guard);

        let mut guard = self.conn_manager.write().await;
        if let Some(cm) = &*guard {
            return Arc::clone(cm);
        }

        let limit = GLOBAL_CONCURRENT_CONNECTIONS
            .get()
            .copied()
            .unwrap_or(DEFAULT_CONCURRENT_CONNECTIONS);
        let cm = Arc::new(crate::connections::ConnectionManager::with_global_limit(
            limit,
        ));
        *guard = Some(Arc::clone(&cm));
        cm
    }
}

/// Module loader for the HTTP reverse proxy module.
#[derive(Default)]
pub struct ReverseProxyModuleLoader {
    _private: (),
}

impl ReverseProxyModuleLoader {
    pub fn new() -> Self {
        Self { _private: () }
    }
}

impl ModuleLoader for ReverseProxyModuleLoader {
    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn ConfigurationValidator>>,
    ) {
        registry.push(Box::new(ProxyConfigurationValidator));
    }

    fn register_stages(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        let state = Arc::new(ProxyState::new());
        registry.with_stage::<HttpContext, _>(move || {
            Arc::new(ReverseProxyStage {
                state: Arc::clone(&state),
            })
        })
    }

    fn register_modules(
        &mut self,
        _registry: Arc<ferron_core::registry::Registry>,
        modules: &mut Vec<Arc<dyn Module>>,
        config: Arc<ferron_core::config::ServerConfiguration>,
    ) -> Result<(), Box<dyn std::error::Error>> {
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
                let _ = GLOBAL_CONCURRENT_CONNECTIONS.set(val as usize);
            }
        }

        modules.push(Arc::new(ReverseProxyModule));
        Ok(())
    }
}

/// The reverse proxy module.
///
/// Responsible for:
/// - Capturing the secondary Tokio runtime handle (for SRV resolution)
struct ReverseProxyModule;

impl Module for ReverseProxyModule {
    fn name(&self) -> &str {
        "reverse-proxy"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn start(&self, runtime: &mut Runtime) -> Result<(), Box<dyn std::error::Error>> {
        // Capture the secondary Tokio runtime handle for SRV lookups
        let _handle = get_secondary_runtime_handle(runtime);
        ferron_core::log_debug!("Reverse proxy module initialized");
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

    fn constraints(&self) -> Vec<ferron_core::StageConstraint> {
        vec![ferron_core::StageConstraint::Before(
            "not_found".to_string(),
        )]
    }

    async fn run(
        &self,
        ctx: &mut HttpContext,
    ) -> Result<bool, ferron_core::pipeline::PipelineError> {
        let entries = ctx.configuration.get_entries("proxy", true);
        if entries.is_empty() {
            return Ok(true);
        }

        let config = match config::parse_proxy_config(ctx) {
            Ok(Some(cfg)) => cfg,
            Ok(None) => return Ok(true),
            Err(e) => {
                ctx.events.emit(ferron_observability::Event::Log(
                    ferron_observability::LogEvent {
                        target: "ferron-proxy",
                        level: ferron_observability::LogLevel::Error,
                        message: format!("Proxy config error: {e}"),
                    },
                ));
                return Ok(true);
            }
        };

        // Set per-upstream local limits (idempotent — only registered once)
        for uc in &config.upstreams {
            let limit = match uc {
                upstream::Upstream::Static(s) => s.limit,
                #[cfg(feature = "srv-lookup")]
                upstream::Upstream::Srv(s) => s.limit,
            };
            if let Some(limit) = limit {
                let resolved = uc
                    .resolve(
                        Arc::clone(&self.state.failed_backends),
                        config.lb_health_check_max_fails,
                    )
                    .await;
                for resolved_upstream in resolved {
                    let conn_manager = self.state.get_conn_manager().await;
                    conn_manager
                        .set_local_limit(&resolved_upstream, limit)
                        .await;
                }
            }
        }

        let algorithm = Arc::new(config.lb_algorithm.into());
        let conn_manager = self.state.get_conn_manager().await;

        let result = proxy::execute_proxy(
            ctx,
            &config,
            &conn_manager,
            Arc::clone(&self.state.failed_backends),
            &algorithm,
            Some(&self.state.conn_state),
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
                    },
                ));
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

        // Emit per-backend unhealthy metrics
        for backend in &metrics.unhealthy_backends {
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
                    name: "ferron.proxy.backends.unhealthy",
                    attributes: attrs,
                    ty: MetricType::Counter,
                    value: MetricValue::U64(1),
                    unit: Some("{backend}"),
                    description: Some("Number of health check failures for a backend server."),
                }));
        }

        // Emit request counter with connection reuse flag
        ctx.events
            .emit(ferron_observability::Event::Metric(MetricEvent {
                name: "ferron.proxy.requests",
                attributes: vec![(
                    "ferron.proxy.connection_reused",
                    MetricAttributeValue::Bool(metrics.connection_reused),
                )],
                ty: MetricType::Counter,
                value: MetricValue::U64(1),
                unit: Some("{request}"),
                description: Some("Number of reverse proxy requests."),
            }));

        Ok(false)
    }

    async fn run_inverse(
        &self,
        _ctx: &mut HttpContext,
    ) -> Result<(), ferron_core::pipeline::PipelineError> {
        Ok(())
    }
}
