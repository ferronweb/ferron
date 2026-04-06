//! HTTP static file serving module for Ferron.
//!
//! Provides pipeline stages for:
//! - `DirectoryIndexStage` — resolves index files (index.html, etc.) in directories
//! - `DirectoryListingStage` — generates HTML directory listings when enabled
//! - `StaticFileStage` — serves files with MIME types, ETags, range requests, and compression
//! - `ErrorPageStage` — serves static HTML files for HTTP error responses

mod stages;
mod util;
mod validator;

use std::sync::Arc;

use ferron_core::loader::ModuleLoader;
use ferron_core::registry::RegistryBuilder;
use ferron_http::{HttpErrorContext, HttpFileContext};

pub use stages::{DirectoryListingStage, ErrorPageStage, StaticFileStage};
pub use validator::HttpStaticConfigurationValidator;

/// Module loader for the HTTP static file module.
///
/// Registers:
/// - Global configuration validator for static file directives
/// - Pipeline stages: DirectoryIndexStage, DirectoryListingStage, StaticFileStage, ErrorPageStage
///
/// Note: This loader does not register any `Module` instances. All functionality
/// is provided through pipeline stages.
#[derive(Default)]
pub struct StaticFileModuleLoader;

impl ModuleLoader for StaticFileModuleLoader {
    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn ferron_core::config::validator::ConfigurationValidator>>,
    ) {
        registry.push(Box::new(HttpStaticConfigurationValidator));
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
            .push(Box::new(HttpStaticConfigurationValidator));
    }

    fn register_stages(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        registry
            .with_stage::<HttpFileContext, _>(|| Arc::new(DirectoryListingStage))
            .with_stage::<HttpFileContext, _>(|| Arc::new(StaticFileStage))
            .with_stage::<HttpErrorContext, _>(|| Arc::new(ErrorPageStage))
    }
}
