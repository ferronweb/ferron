//! HTTP response cache with LSCache-compatible response header controls.

mod config;
mod lscache;
mod policy;
mod stage;
mod store;
mod validator;

use std::sync::Arc;

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::loader::ModuleLoader;
use ferron_core::registry::RegistryBuilder;
use ferron_http::HttpContext;

pub use stage::HttpCacheStage;
pub use validator::{HttpCacheConfigurationValidator, HttpCacheGlobalConfigurationValidator};

/// Module loader for the HTTP cache module.
#[derive(Default)]
pub struct HttpCacheModuleLoader;

impl ModuleLoader for HttpCacheModuleLoader {
    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn ConfigurationValidator>>,
    ) {
        registry.push(Box::new(HttpCacheGlobalConfigurationValidator));
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
            .push(Box::new(HttpCacheConfigurationValidator));
    }

    fn register_stages(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        let stage = Arc::new(HttpCacheStage::new());
        registry.with_stage::<HttpContext, _>(move || stage.clone())
    }
}
