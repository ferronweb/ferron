//! HTTP response cache with LSCache-compatible response header controls.

#![cfg_attr(feature = "fuzz", allow(private_interfaces))]

mod config;
#[cfg(feature = "fuzz")]
pub mod lscache;
#[cfg(not(feature = "fuzz"))]
mod lscache;
#[cfg(feature = "fuzz")]
pub mod policy;
#[cfg(not(feature = "fuzz"))]
mod policy;
mod stage;
#[cfg(feature = "fuzz")]
pub mod store;
#[cfg(not(feature = "fuzz"))]
mod store;
mod validator;

use std::sync::Arc;

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::loader::ModuleLoader;
use ferron_core::registry::RegistryBuilder;
use ferron_http::HttpContext;

pub use stage::HttpCacheStage;
pub use validator::HttpCacheConfigurationValidator;

/// Module loader for the HTTP cache module.
#[derive(Default)]
pub struct HttpCacheModuleLoader;

impl ModuleLoader for HttpCacheModuleLoader {
    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn ConfigurationValidator>>,
    ) {
        registry.push(Box::new(HttpCacheConfigurationValidator));
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
