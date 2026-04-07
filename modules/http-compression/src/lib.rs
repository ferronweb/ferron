//! HTTP dynamic compression module for Ferron.
//!
//! Provides a pipeline stage for on-the-fly response body compression
//! based on the client's `Accept-Encoding` header.
//!
//! Supported algorithms: gzip, brotli, deflate, zstd.

mod stages;
mod validator;

use std::sync::Arc;

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::loader::ModuleLoader;
use ferron_core::registry::RegistryBuilder;
use ferron_http::HttpContext;

pub use stages::DynamicCompressionStage;
pub use validator::DynamicCompressionConfigurationValidator;

/// Module loader for the HTTP dynamic compression module.
///
/// Registers:
/// - Per-protocol configuration validator for the `dynamic_compressed` directive
/// - Pipeline stage: DynamicCompressionStage
///
/// Note: This loader does not register any `Module` instances. All functionality
/// is provided through pipeline stages.
#[derive(Default)]
pub struct HttpCompressionModuleLoader;

impl ModuleLoader for HttpCompressionModuleLoader {
    fn register_global_configuration_validators(
        &mut self,
        _registry: &mut Vec<Box<dyn ConfigurationValidator>>,
    ) {
        // No global validators — dynamic_compressed is a per-host directive
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
            .push(Box::new(DynamicCompressionConfigurationValidator));
    }

    fn register_stages(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        registry.with_stage::<HttpContext, _>(|| Arc::new(DynamicCompressionStage::new()))
    }
}
