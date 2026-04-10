//! Admin API server module.
//!
//! Spawns a standalone axum HTTP server on a secondary Tokio runtime
//! for administrative endpoints.

mod router;

use std::sync::Arc;

use arc_swap::ArcSwap;
use ferron_core::config::ServerConfiguration;
use ferron_core::registry::Registry;
use ferron_core::runtime::Runtime;
use ferron_core::Module;
use tokio_util::sync::CancellationToken;

use crate::config::AdminConfig;
use crate::handlers::AdminState;
use crate::server::router::build_admin_router;

/// Admin API module implementing the `Module` trait.
///
/// Runs a separate axum HTTP server on a configurable port
/// using a secondary Tokio runtime (control plane isolation).
pub struct AdminApiModule {
    /// Atomic swap for admin config (endpoint enable/disable, listen address).
    config: Arc<ArcSwap<AdminConfig>>,
    /// Full server configuration, used by the `/config` endpoint.
    full_config: Arc<ServerConfiguration>,
    /// Token cancelled on reload to gracefully shut down the admin listener.
    reload_token: ArcSwap<CancellationToken>,
}

impl AdminApiModule {
    /// Create a new admin API module.
    pub fn new(
        _registry: &Arc<Registry>,
        admin_config: AdminConfig,
        full_config: Arc<ServerConfiguration>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            config: Arc::new(ArcSwap::new(Arc::new(admin_config))),
            full_config,
            reload_token: ArcSwap::from_pointee(CancellationToken::new()),
        })
    }

    /// Reload the module with new admin configuration.
    ///
    /// Called during configuration reload (SIGHUP) when the `admin {}` block changes.
    /// Atomically replaces the config and cancels the old reload token to gracefully
    /// shut down the existing listener.
    pub fn reload(
        &self,
        _registry: &Arc<Registry>,
        admin_config: AdminConfig,
        _full_config: Arc<ServerConfiguration>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Cancel the old reload token to trigger graceful shutdown
        let old_token = self.reload_token.load();
        old_token.cancel();

        // Atomically swap the config
        self.config.store(Arc::new(admin_config));

        // Create a new reload token
        self.reload_token.store(Arc::new(CancellationToken::new()));

        Ok(())
    }
}

impl Module for AdminApiModule {
    fn name(&self) -> &str {
        "admin-api"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn start(&self, runtime: &mut Runtime) -> Result<(), Box<dyn std::error::Error>> {
        let config = self.config.load_full();
        let full_config = self.full_config.clone();

        // Clone the CancellationToken so the spawned task owns it
        let reload_token = (*self.reload_token.load_full()).clone();

        // Spawn on secondary runtime — control plane isolation
        runtime.spawn_secondary_task(async move {
            let state = AdminState { full_config };
            let app = build_admin_router(&config, state);

            match tokio::net::TcpListener::bind(config.listen).await {
                Ok(listener) => {
                    ferron_core::log_info!("Admin API listening on {}", config.listen);
                    let server = axum::serve(listener, app);
                    tokio::select! {
                        _ = reload_token.cancelled() => {
                            ferron_core::log_info!("Admin API shutting down (reload)");
                        }
                        result = server => {
                            if let Err(e) = result {
                                ferron_core::log_error!("Admin API server error: {}", e);
                            }
                        }
                    }
                }
                Err(e) => {
                    ferron_core::log_error!(
                        "Failed to bind admin API listener on {}: {}",
                        config.listen,
                        e
                    );
                }
            }
        });

        Ok(())
    }
}

impl Drop for AdminApiModule {
    fn drop(&mut self) {
        self.reload_token.load().cancel();
    }
}
