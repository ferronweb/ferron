//! HTTP response control module.
//!
//! Provides directives for returning custom status codes, aborting connections,
//! IP-based access control, and 103 Early Hints.
//!
//! ## Supported Directives
//!
//! - `abort true` — Immediately close the connection without a response
//! - `block "ip" "cidr"` — Block listed IPs/CIDRs
//! - `allow "ip" "cidr"` — Allow listed IPs/CIDRs only
//! - `status <code> { url|regex|body|location }` — Return a custom status code
//! - `early_hints { link "..." }` — Send 103 Early Hints with Link headers

mod config;
mod stage;
mod validator;

pub use stage::{EarlyHintsStage, HttpResponseStage, ResponseEngine};
pub use validator::HttpResponseValidator;

use std::collections::HashMap;
use std::sync::Arc;

use ferron_core::loader::ModuleLoader;
use ferron_core::registry::RegistryBuilder;

/// Module loader for the http-response module.
#[derive(Default)]
pub struct HttpResponseModuleLoader;

impl ModuleLoader for HttpResponseModuleLoader {
    fn register_directives(&mut self, registry: &mut ferron_core::directives::DirectiveRegistry) {
        use ferron_core::directives::{Directive, DirectiveSubblock};
        registry
            .register(
                Directive {
                    name: "abort",
                    usage: "abort [bool]",
                    description: "This directive closes the connection immediately without sending a response.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "block",
                    usage: "block <ip-or-cidr>...",
                    description: "This directive denies access to requests from specified IPs or CIDR ranges.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "allow",
                    usage: "allow <ip-or-cidr>...",
                    description: "This directive allows access from specified IPs or CIDR ranges, bypassing block rules.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "status",
                    usage: "status <code> { ... }",
                    description: "This directive returns a custom HTTP status code with optional url, regex, location, and body rules.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: Some(DirectiveSubblock::custom("http_status")),
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "url",
                    usage: "url <path>",
                    description: "This directive sets the URL path for a custom status rule.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_status"),
            )
            .register(
                Directive {
                    name: "regex",
                    usage: "regex <pattern>",
                    description: "This directive sets a regex pattern for matching request paths in custom status rules.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_status"),
            )
            .register(
                Directive {
                    name: "location",
                    usage: "location <url>",
                    description: "This directive sets the redirect location for 3xx custom status responses.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_status"),
            )
            .register(
                Directive {
                    name: "body",
                    usage: "body <content>",
                    description: "This directive sets the response body for custom status responses.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_status"),
            );
    }

    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn ferron_core::config::validator::ConfigurationValidator>>,
    ) {
        registry.push(Box::new(HttpResponseValidator));
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
            .push(Box::new(HttpResponseValidator));
    }

    fn register_stages(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        let engine = Arc::new(ResponseEngine::new());
        let engine2 = engine.clone();
        registry
            .with_stage::<ferron_http::HttpContext, _>(move || {
                Arc::new(HttpResponseStage::new(engine.clone()))
            })
            .with_stage::<ferron_http::HttpContext, _>(move || {
                Arc::new(EarlyHintsStage::new(engine2.clone()))
            })
    }
}
