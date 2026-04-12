//! HTTP response body string replacement module for Ferron.
//!
//! Provides the `replace`, `replace_last_modified`, and `replace_filter_types`
//! directives for modifying response bodies on the fly.
//!
//! ## Supported Directives
//!
//! - `replace "search" "replacement" { once true|false }` — Replace strings in response bodies
//! - `replace_last_modified true|false` — Whether to preserve the Last-Modified header
//! - `replace_filter_types "mime/type" ...` — MIME types to process (default: text/html)

mod body_replacer;
mod config;
mod stage;
mod validator;

pub use stage::HttpReplaceStage;
pub use validator::ReplaceConfigurationValidator;

use std::collections::HashMap;
use std::sync::Arc;

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::loader::ModuleLoader;
use ferron_core::registry::RegistryBuilder;
use ferron_http::HttpContext;

/// Module loader for the HTTP replace module.
///
/// Registers:
/// - Per-protocol configuration validator for replace directives
/// - Pipeline stage: HttpReplaceStage
///
/// Note: This loader does not register any `Module` instances. All functionality
/// is provided through pipeline stages.
#[derive(Default)]
pub struct HttpReplaceModuleLoader;

impl ModuleLoader for HttpReplaceModuleLoader {
    fn register_global_configuration_validators(
        &mut self,
        _registry: &mut Vec<Box<dyn ConfigurationValidator>>,
    ) {
        // No global validators — replace directives are per-host
    }

    fn register_per_protocol_configuration_validators(
        &mut self,
        registry: &mut HashMap<&'static str, Vec<Box<dyn ConfigurationValidator>>>,
    ) {
        registry
            .entry("http")
            .or_default()
            .push(Box::new(ReplaceConfigurationValidator));
    }

    fn register_stages(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        registry.with_stage::<HttpContext, _>(|| Arc::new(HttpReplaceStage::new()))
    }
}
