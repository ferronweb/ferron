//! HTTP reverse proxy module for Ferron.
#![cfg_attr(feature = "fuzz", allow(private_interfaces))]

mod config;
mod connections;
pub(crate) mod directives;
mod health_check;
pub(crate) mod metrics;
mod per_config;
mod proxy;
pub(crate) mod runtime_handle;
mod send_net_io;
mod send_request;
mod stage;
#[cfg(feature = "fuzz")]
pub mod types;
#[cfg(not(feature = "fuzz"))]
mod types;
#[cfg(feature = "fuzz")]
pub mod upstream;
#[cfg(not(feature = "fuzz"))]
mod upstream;
mod validator;

use std::collections::HashMap;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use arc_swap::ArcSwap;
use dashmap::DashMap;
use ferron_observability::build_composite_sink;
use parking_lot::RwLock;
use rustc_hash::FxBuildHasher;
use types::retry_budget::SharedRetryBudget;

use crate::per_config::{PerConfigCache, TaskRegistry};
use crate::stage::ReverseProxyStage;
use crate::types::upstream::Upstream;
use crate::types::ConnectionsTrackState;
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

pub use metrics::ProxyMetrics;

/// Shared counter type for tracking active health check unhealthy events.
type ActiveUnhealthyCounters = parking_lot::RwLock<std::collections::HashMap<String, u64>>;

const DEFAULT_CONCURRENT_CONNECTIONS: usize = 16384;
const LOG_TARGET: &str = "ferron-http-proxy";

/// Global concurrent connections limit, read from config during `register_modules`.
/// Uses `AtomicUsize` to allow updates during config reload.
static GLOBAL_CONCURRENT_CONNECTIONS: AtomicUsize =
    AtomicUsize::new(DEFAULT_CONCURRENT_CONNECTIONS);

/// Shared state for the reverse proxy stage, constructed once and reused
/// across all requests to preserve connection pools, health tracking,
/// and the load balancer algorithm (which must be shared for RoundRobin to work).
struct ProxyState {
    /// Connection pool manager, lazily initialized on first use so we can
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
    /// Flapping detection state per upstream.
    flapping_state: types::flapping::FlappingStateMap,
    /// Background health check task handles, keyed by configuration pointer.
    /// Tasks are aborted on reload via `on_reload`.
    health_check_tasks: TaskRegistry,
    /// Counters for active health check unhealthy events, keyed by configuration pointer.
    active_unhealthy_counters: PerConfigCache<Arc<ActiveUnhealthyCounters>>,
    /// Whether to include resolved IP addresses in proxy metrics attributes.
    /// Updated from config on each request.
    metrics_resolved_ip: std::sync::atomic::AtomicBool,
    /// Retry budget state, keyed by config pointer identity.
    retry_budget_states: PerConfigCache<SharedRetryBudget>,
}

impl ProxyState {
    #[inline]
    fn new() -> Self {
        Self {
            conn_manager: RwLock::new(None),
            circuit_breaker_state: Arc::new(DashMap::with_hasher(FxBuildHasher)),
            conn_state: Arc::new(DashMap::with_hasher(FxBuildHasher)),
            ewma_state: Arc::new(DashMap::with_hasher(FxBuildHasher)),
            algorithms: ArcSwap::from_pointee(DashMap::with_hasher(FxBuildHasher)),
            active_health_check_state: Arc::new(DashMap::with_hasher(FxBuildHasher)),
            flapping_state: Arc::new(DashMap::with_hasher(FxBuildHasher)),
            health_check_tasks: TaskRegistry::new(),
            active_unhealthy_counters: PerConfigCache::new(),
            metrics_resolved_ip: std::sync::atomic::AtomicBool::new(false),
            retry_budget_states: PerConfigCache::new(),
        }
    }

    /// Invalidate all per-config state on config reload.
    ///
    /// A reloaded config arrives under fresh layer Arc pointer keys, so the
    /// previous generation must be discarded here: health check probe loops
    /// are aborted first (no task keeps probing after its config is gone),
    /// then the caches are cleared, then the load-balancer algorithms are
    /// swapped for the new generation.
    #[inline]
    fn on_reload(&self) {
        self.health_check_tasks.abort_all();
        self.retry_budget_states.clear();
        // Since active health check state is set on first (immediately),
        // and later (after delays) health checks, let's just clear it all.
        self.active_health_check_state.clear();
        self.active_unhealthy_counters.clear();
        self.algorithms.swap(Default::default());
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
            Upstream::Srv(cfg) => cfg.health_check_config.enabled,
        });

        if !has_health_checks {
            return;
        }

        // Get the secondary runtime handle for spawning the health check task
        let (runtime_handle, event_sink) = match runtime_handle::try_get_secondary_runtime_handle()
        {
            Some(h) => h,
            None => {
                ferron_core::log_warn!(
                    "Health check task not spawned — secondary runtime not yet available"
                );
                return;
            }
        };

        // Spawn the health check task with a callback to update the shared counter
        self.health_check_tasks.ensure(config_keys, || {
            let counter = self
                .active_unhealthy_counters
                .get_or_insert_with(config_keys, || {
                    Arc::new(ActiveUnhealthyCounters::new(HashMap::new()))
                });
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

            task.abort_handle()
        });
    }
}

/// Module loader for the HTTP reverse proxy module.
#[derive(Default)]
pub struct ReverseProxyModuleLoader {
    /// Shared proxy state, set during `register_stages` and used in `register_modules`.
    state: Option<Arc<ProxyState>>,
}

impl ModuleLoader for ReverseProxyModuleLoader {
    #[inline]
    fn register_directives(&mut self, registry: &mut ferron_core::directives::DirectiveRegistry) {
        directives::register_core_proxy_directives(registry);
        directives::register_health_check_directives(registry);
        directives::register_upstream_connection_directives(registry);
        directives::register_circuit_breaker_directives(registry);
        directives::register_retry_budget_directives(registry);
        directives::register_connection_feature_directives(registry);
        directives::register_affinity_directives(registry);
    }

    #[inline]
    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn ConfigurationValidator>>,
    ) {
        registry.push(Box::new(ProxyConfigurationValidator));
    }

    #[inline]
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

    #[inline]
    fn register_stages(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        let state = Arc::new(ProxyState::new());
        self.state = Some(Arc::clone(&state));
        registry.with_stage::<HttpContext, _>(move || {
            Arc::new(ReverseProxyStage {
                state: Arc::clone(&state),
            })
        })
    }

    #[inline]
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

        // Prevent load balancing state memory leaks on config reload
        if let Some(ref state) = self.state {
            state.on_reload();
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
    #[inline]
    fn name(&self) -> &str {
        "http-proxy"
    }

    #[inline]
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    #[inline]
    fn start(&self, runtime: &mut Runtime) -> Result<(), Box<dyn std::error::Error>> {
        // Capture the secondary Tokio runtime handle for SRV lookups and pool gauge emission
        let (secondary_handle, pool_sink) =
            runtime_handle::get_secondary_runtime_handle(runtime, self.sink.clone());

        // Spawn periodic pool depth gauge emission on the secondary runtime
        secondary_handle.spawn(metrics::emit_pool_and_dns_metrics(pool_sink));
        // Spawn periodic DNS result cache cleanup on the secondary runtime
        secondary_handle.spawn(metrics::cleanup_dns_cache_task());

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
