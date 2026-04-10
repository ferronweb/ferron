//! Module loader implementation for HTTP rate limiting.

use std::collections::HashMap;
use std::sync::Arc;

use ferron_core::loader::ModuleLoader;
use ferron_core::registry::RegistryBuilder;

use crate::stage::{RateLimitEngine, RateLimitStage};
use crate::validator::RateLimitValidator;

#[derive(Default)]
pub struct HttpRateLimitModuleLoader;

impl ModuleLoader for HttpRateLimitModuleLoader {
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
            .push(Box::new(RateLimitValidator));
    }

    fn register_stages(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        let engine = Arc::new(RateLimitEngine::new());
        registry.with_stage::<ferron_http::HttpContext, _>(move || {
            Arc::new(RateLimitStage::new(engine.clone()))
        })
    }
}
