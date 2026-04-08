//! OCSP stapler module for Ferron.
//!
//! This module initializes the OCSP stapling service on the secondary tokio
//! runtime during server startup. It is a thin `ModuleLoader` shim — the actual
//! OCSP logic lives in the `ferron-ocsp` crate.
//!
//! Once loaded, any TLS provider can use `ferron_ocsp::get_service_handle()` to
//! wrap its certificate resolver with `OcspStapler` for automatic OCSP stapling.

use std::sync::Arc;

use ferron_core::{loader::ModuleLoader, log_debug, registry::Registry, Module};
use ferron_observability::build_composite_sink;

struct OcspStaplerModule {
    event_sink: Arc<ferron_observability::CompositeEventSink>,
}

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
        // Configure the event sink for the OCSP service before initialization
        ferron_ocsp::set_event_sink(self.event_sink.clone());

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
        registry: Arc<Registry>,
        modules: &mut Vec<Arc<dyn ferron_core::Module>>,
        config: Arc<ferron_core::config::ServerConfiguration>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Build the composite event sink from observability providers
        let event_sink = build_composite_sink(&registry, &config.global_config)?;
        modules.push(Arc::new(OcspStaplerModule { event_sink }));
        Ok(())
    }
}
