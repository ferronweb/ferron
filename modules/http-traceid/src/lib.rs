mod config;
mod stage;
mod validator;

pub use stage::HttpTraceIdStage;
pub use validator::HttpTraceIdConfigurationValidator;

use std::sync::Arc;

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::loader::ModuleLoader;
use ferron_core::registry::RegistryBuilder;
use ferron_http::HttpContext;

#[derive(Default)]
pub struct HttpTraceIdModuleLoader;

impl ModuleLoader for HttpTraceIdModuleLoader {
    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn ConfigurationValidator>>,
    ) {
        registry.push(Box::new(HttpTraceIdConfigurationValidator));
    }

    fn register_per_protocol_configuration_validators(
        &mut self,
        registry: &mut std::collections::HashMap<
            &'static str,
            Vec<Box<dyn ConfigurationValidator>>,
        >,
    ) {
        registry
            .entry("http")
            .or_default()
            .push(Box::new(HttpTraceIdConfigurationValidator));
    }

    fn register_stages(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        registry.with_stage::<HttpContext, _>(|| Arc::new(HttpTraceIdStage::new()))
    }
}
