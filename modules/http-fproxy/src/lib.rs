//! Forward proxy module for Ferron.
//!
//! Provides pipeline stages for forward proxying with:
//! - HTTP CONNECT tunneling (for HTTPS/WebSocket)
//! - HTTP/1.x absolute URI forwarding
//! - ACL-based access control (domain allowlisting, port allowlisting, IP denylisting)

mod config;
mod proxy;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::OnceLock;

use ferron_core::loader::ModuleLoader;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::registry::RegistryBuilder;
use ferron_core::runtime::Runtime;
use ferron_core::Module;
use ferron_http::HttpContext;

pub use config::ForwardProxyConfig;
pub use config::ForwardProxyConfigurationValidator;

/// Global accessor for the secondary Tokio runtime handle.
///
/// Populated during `ForwardProxyModule::start()` by spawning a task
/// that captures `tokio::runtime::Handle::current()`.
/// Used for DNS resolution via `tokio::net::lookup_host`.
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

/// Module loader for the HTTP forward proxy module.
#[derive(Default)]
pub struct ForwardProxyModuleLoader;

impl ModuleLoader for ForwardProxyModuleLoader {
    fn register_per_protocol_configuration_validators(
        &mut self,
        registry: &mut HashMap<
            &'static str,
            Vec<Box<dyn ferron_core::config::validator::ConfigurationValidator>>,
        >,
    ) {
        registry
            .entry("http")
            .or_default()
            .push(Box::new(ForwardProxyConfigurationValidator));
    }

    fn register_stages(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        registry.with_stage::<HttpContext, _>(|| Arc::new(ForwardProxyStage))
    }

    fn register_modules(
        &mut self,
        _registry: Arc<ferron_core::registry::Registry>,
        modules: &mut Vec<Arc<dyn Module>>,
        _config: Arc<ferron_core::config::ServerConfiguration>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        modules.push(Arc::new(ForwardProxyModule));
        Ok(())
    }
}

/// The forward proxy module.
struct ForwardProxyModule;

impl Module for ForwardProxyModule {
    fn name(&self) -> &str {
        "forward-proxy"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn start(&self, runtime: &mut Runtime) -> Result<(), Box<dyn std::error::Error>> {
        // Capture the secondary runtime handle for DNS resolution
        let _handle = get_secondary_runtime_handle(runtime);
        ferron_core::log_debug!("Forward proxy module initialized");
        Ok(())
    }
}

/// Pipeline stage for forward proxy handling.
struct ForwardProxyStage;

#[async_trait::async_trait(?Send)]
impl Stage<HttpContext> for ForwardProxyStage {
    fn name(&self) -> &str {
        "forward_proxy"
    }

    fn constraints(&self) -> Vec<ferron_core::StageConstraint> {
        vec![ferron_core::StageConstraint::Before(
            "reverse_proxy".to_string(),
        )]
    }

    fn is_applicable(
        &self,
        config: Option<&ferron_core::config::ServerConfigurationBlock>,
    ) -> bool {
        config.is_some_and(|c| c.has_directive("forward_proxy"))
    }

    #[inline]
    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        let config = match config::parse_forward_proxy_config(ctx) {
            Ok(Some(cfg)) => cfg,
            Ok(None) => return Ok(true),
            Err(e) => {
                ferron_core::log_error!("Forward proxy config error: {e}");
                return Ok(true);
            }
        };

        match proxy::execute_forward_proxy(ctx, &config).await {
            Ok(proxy::ForwardProxyResult::Handled) => Ok(false),
            Ok(proxy::ForwardProxyResult::PassThrough) => Ok(true),
            Err(e) => {
                ferron_core::log_error!("Forward proxy error: {e}");
                // If we have a response already set, stop; otherwise continue
                if ctx.res.is_some() {
                    Ok(false)
                } else {
                    Ok(true)
                }
            }
        }
    }

    #[inline]
    async fn run_inverse(&self, _ctx: &mut HttpContext) -> Result<(), PipelineError> {
        Ok(())
    }
}
