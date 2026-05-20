//! Module loader implementation for HTTP Basic Authentication.

use std::collections::HashMap;
use std::sync::Arc;

use ferron_core::loader::ModuleLoader;
use ferron_core::registry::RegistryBuilder;

use crate::stage::{BasicAuthStage, GLOBAL_CONCURRENCY_SEMAPHORE};
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
        registry.with_stage::<ferron_http::HttpContext, _>(move || Arc::new(BasicAuthStage::new()))
    }

    fn register_modules(
        &mut self,
        _registry: Arc<ferron_core::registry::Registry>,
        _modules: &mut Vec<Arc<dyn ferron_core::Module>>,
        config: Arc<ferron_core::config::ServerConfiguration>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let basic_auth_concurrency = config
            .global_config
            .get_value("basic_auth_concurrency")
            .and_then(|v| {
                if v.as_boolean() == Some(false) {
                    None
                } else {
                    Some(v.as_number().unwrap_or(128).max(1) as usize)
                }
            });

        *GLOBAL_CONCURRENCY_SEMAPHORE.blocking_write() =
            basic_auth_concurrency.map(|basic_auth_concurrency| {
                Arc::new(tokio::sync::Semaphore::new(basic_auth_concurrency))
            });

        Ok(())
    }
}
