//! JSON error response module for Ferron.
//!
//! Provides a pipeline stage for the error pipeline (`HttpErrorContext`)
//! that generates structured JSON error responses (RFC 9457 Problem Details
//! or simple JSON) instead of HTML error pages.

mod config;
mod stage;
mod validator;

use std::collections::HashMap;
use std::sync::Arc;

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::loader::ModuleLoader;
use ferron_core::registry::RegistryBuilder;
use ferron_http::HttpErrorContext;

pub use stage::JsonErrorStage;
pub use validator::JsonErrorConfigurationValidator;

/// Module loader for the JSON error response module.
#[derive(Default)]
pub struct HttpJsonErrorModuleLoader;

impl ModuleLoader for HttpJsonErrorModuleLoader {
    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn ConfigurationValidator>>,
    ) {
        registry.push(Box::new(JsonErrorConfigurationValidator));
    }

    fn register_per_protocol_configuration_validators(
        &mut self,
        registry: &mut HashMap<&'static str, Vec<Box<dyn ConfigurationValidator>>>,
    ) {
        registry
            .entry("http")
            .or_default()
            .push(Box::new(JsonErrorConfigurationValidator));
    }

    fn register_stages(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        registry.with_stage::<HttpErrorContext, _>(|| Arc::new(JsonErrorStage))
    }
}
