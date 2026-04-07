//! Module loader implementation for HTTP Basic Authentication.

use std::collections::HashMap;
use std::sync::Arc;

use ferron_core::loader::ModuleLoader;
use ferron_core::registry::RegistryBuilder;

use crate::brute_force::{BruteForceConfig, BruteForceEngine};
use crate::stage::BasicAuthStage;
use crate::validator::BasicAuthValidator;

#[derive(Default)]
pub struct HttpBasicAuthModuleLoader;

impl ModuleLoader for HttpBasicAuthModuleLoader {
    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn ferron_core::config::validator::ConfigurationValidator>>,
    ) {
        registry.push(Box::new(BasicAuthValidator));
    }

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
            .push(Box::new(BasicAuthValidator));
    }

    fn register_stages(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        // Create a shared brute-force engine with default config.
        // The actual per-request config (max_attempts, window, etc.) is read from
        // the LayeredConfiguration at runtime, but the engine itself needs initial defaults.
        let engine = Arc::new(BruteForceEngine::new(BruteForceConfig::default()));
        registry.with_stage::<ferron_http::HttpContext, _>(move || {
            Arc::new(BasicAuthStage::new(engine.clone()))
        })
    }
}
