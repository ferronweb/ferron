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

    fn register_directives(&mut self, registry: &mut ferron_core::directives::DirectiveRegistry) {
        use ferron_core::directives::{Directive, DirectiveSubblock};
        registry
            .register(
                Directive {
                    name: "json_errors",
                    usage: "json_errors [bool] | json_errors { ... }",
                    description: "This directive enables JSON error responses with optional format, type URI, and trace ID settings.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: Some(DirectiveSubblock::custom("http_json_errors")),
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "format",
                    usage: "format <type>",
                    description: "This directive sets the JSON error response format. Supported: problem, simple.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_json_errors"),
            )
            .register(
                Directive {
                    name: "type_uri",
                    usage: "type_uri <uri>",
                    description: "This directive sets the type URI for RFC 9457 Problem Details error responses.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_json_errors"),
            )
            .register(
                Directive {
                    name: "trace_id",
                    usage: "trace_id [bool]",
                    description: "This directive enables trace ID inclusion in JSON error responses.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_json_errors"),
            );
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
