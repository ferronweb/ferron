//! HTTP static file serving module for Ferron.
//!
//! Provides pipeline stages for:
//! - `DirectoryIndexStage`: resolves index files (index.html, etc.) in directories
//! - `DirectoryListingStage`: generates HTML directory listings when enabled
//! - `StaticFileStage`: serves files with MIME types, ETags, range requests, and compression
//! - `ErrorPageStage`: serves static HTML files for HTTP error responses

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
    fn register_directives(&mut self, registry: &mut ferron_core::directives::DirectiveRegistry) {
        use ferron_core::directives::{Directive, DirectiveSubblock};
        registry
            .register(
                Directive {
                    name: "compressed",
                    usage: "compressed [bool]",
                    description: "This directive enables on-the-fly compression for static files using Zstandard, Brotli, gzip, or Deflate.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "precompressed",
                    usage: "precompressed [bool]",
                    description: "This directive enables serving pre-compressed static files (.zst, .br, .gz, .zz).",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "etag",
                    usage: "etag [bool]",
                    description: "This directive enables ETag header generation for static files.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "directory_listing",
                    usage: "directory_listing [bool]",
                    description: "This directive enables automatic directory listing when no index file is found.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "file_cache_control",
                    usage: "file_cache_control <value>",
                    description: "This directive sets the Cache-Control header for static file responses.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "mime_type",
                    usage: "mime_type <.ext> <type>",
                    description: "This directive maps a file extension to a MIME type.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "error_page",
                    usage: "error_page <code>... <path>",
                    description: "This directive maps HTTP error status codes to custom error page files.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "error_page_placeholders",
                    usage: "error_page_placeholders [bool]",
                    description: "This directive enables placeholder variable substitution in error pages.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::default(),
            );
    }

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
