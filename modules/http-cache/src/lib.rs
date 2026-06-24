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

use std::sync::{Arc, OnceLock};

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::loader::ModuleLoader;
use ferron_core::registry::RegistryBuilder;
use ferron_http::HttpContext;

pub use stage::HttpCacheStage;
pub use validator::HttpCacheConfigurationValidator;

pub static SECONDARY_RUNTIME: OnceLock<tokio::runtime::Handle> = OnceLock::new();

/// Module loader for the HTTP cache module.
#[derive(Default)]
pub struct HttpCacheModuleLoader {
    cache: Option<Arc<HttpCacheModule>>,
}

impl ModuleLoader for HttpCacheModuleLoader {
    #[inline]
    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn ConfigurationValidator>>,
    ) {
        registry.push(Box::new(HttpCacheConfigurationValidator));
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
            .push(Box::new(HttpCacheConfigurationValidator));
    }

    #[inline]
    fn register_stages(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        let stage = Arc::new(HttpCacheStage::new());
        registry.with_stage::<HttpContext, _>(move || stage.clone())
    }

    #[inline]
    fn register_modules(
        &mut self,
        _registry: Arc<ferron_core::registry::Registry>,
        modules: &mut Vec<Arc<dyn ferron_core::Module>>,
        _config: Arc<ferron_core::config::ServerConfiguration>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if self.cache.is_none() {
            let module = Arc::new(HttpCacheModule);
            modules.push(module.clone());
            self.cache = Some(module);
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct HttpCacheModule;

impl ferron_core::Module for HttpCacheModule {
    #[inline]
    fn name(&self) -> &str {
        "http-cache"
    }

    #[inline]
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    #[inline]
    fn start(
        &self,
        runtime: &mut ferron_core::runtime::Runtime,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let _ = SECONDARY_RUNTIME
            .set(runtime.block_on(async move { tokio::runtime::Handle::current() }));
        Ok(())
    }
}
