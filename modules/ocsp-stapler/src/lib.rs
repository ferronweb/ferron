//! OCSP stapler module for Ferron.
//!
//! This module initializes the OCSP stapling service on the secondary tokio
//! runtime during server startup. It is a thin `ModuleLoader` shim — the actual
//! OCSP logic lives in the `ferron-ocsp` crate.
//!
//! Once loaded, any TLS provider can use `ferron_ocsp::get_service_handle()` to
//! wrap its certificate resolver with `OcspStapler` for automatic OCSP stapling.

use std::sync::Arc;

use ferron_core::{loader::ModuleLoader, log_debug, Module};

struct OcspStaplerModule;

impl Module for OcspStaplerModule {
    fn name(&self) -> &str {
        "ocsp-stapler"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn start(
        &self,
        runtime: &mut ferron_core::runtime::Runtime,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match ferron_ocsp::init_ocsp_service(runtime) {
            Ok(()) => log_debug!("OCSP stapling service initialized"),
            Err(ferron_ocsp::AlreadyInitialized) => {
                log_debug!("OCSP stapling service already running (reusing existing instance)")
            }
        }
        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct OcspStaplerModuleLoader;

impl ModuleLoader for OcspStaplerModuleLoader {
    fn register_modules(
        &mut self,
        _registry: Arc<ferron_core::registry::Registry>,
        modules: &mut Vec<Arc<dyn ferron_core::Module>>,
        _config: &mut ferron_core::config::ServerConfiguration,
    ) -> Result<(), Box<dyn std::error::Error>> {
        modules.push(Arc::new(OcspStaplerModule));
        Ok(())
    }
}
