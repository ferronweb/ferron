use std::sync::Arc;

use ferron_http::HttpContext;

use crate::stage::ScgiStage;

mod config;
mod stage;
mod util;
mod validator;

pub struct ScgiModuleLoader;

impl ferron_core::loader::ModuleLoader for ScgiModuleLoader {
    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn ferron_core::config::validator::ConfigurationValidator>>,
    ) {
        registry.push(Box::new(validator::ScgiConfigurationValidator));
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
            .push(Box::new(validator::ScgiConfigurationValidator));
    }

    fn register_stages(
        &mut self,
        registry: ferron_core::registry::RegistryBuilder,
    ) -> ferron_core::registry::RegistryBuilder {
        registry.with_stage::<HttpContext, _>(|| Arc::new(ScgiStage))
    }
}
