//! HTTP request and response buffering module for Ferron.
//!
//! Provides the `buffer_request` and `buffer_response` directives for
//! configuring body buffering at the HTTP pipeline level. This can serve
//! as protection against Slowloris-style attacks and help control memory
//! usage for large request/response bodies.

mod stage;
mod validator;

pub use stage::HttpBufferStage;
pub use validator::HttpBufferConfigurationValidator;

use std::sync::Arc;

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::loader::ModuleLoader;
use ferron_core::registry::RegistryBuilder;
use ferron_http::HttpContext;

/// Module loader for the HTTP buffer module.
///
/// Registers:
/// - Global configuration validator for buffer directives
/// - Per-protocol (HTTP) configuration validator
/// - Pipeline stage: HttpBufferStage
///
/// Note: This loader does not register any `Module` instances. All functionality
/// is provided through pipeline stages.
#[derive(Default)]
pub struct HttpBufferModuleLoader;

impl ModuleLoader for HttpBufferModuleLoader {
    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn ConfigurationValidator>>,
    ) {
        registry.push(Box::new(HttpBufferConfigurationValidator));
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
            .push(Box::new(HttpBufferConfigurationValidator));
    }

    fn register_stages(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        registry.with_stage::<HttpContext, _>(|| Arc::new(HttpBufferStage::new()))
    }
}
