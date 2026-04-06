//! HTTP response control module.
//!
//! Provides directives for returning custom status codes, aborting connections,
//! and IP-based access control.
//!
//! ## Supported Directives
//!
//! - `abort true` — Immediately close the connection without a response
//! - `block "ip" "cidr"` — Block listed IPs/CIDRs
//! - `allow "ip" "cidr"` — Allow listed IPs/CIDRs only
//! - `status <code> { url|regex|body|location }` — Return a custom status code

mod config;
mod stage;
mod validator;

pub use stage::HttpResponseStage;
pub use stage::ResponseEngine;
pub use validator::HttpResponseValidator;

use std::collections::HashMap;
use std::sync::Arc;

use ferron_core::loader::ModuleLoader;
use ferron_core::registry::RegistryBuilder;

/// Module loader for the http-response module.
#[derive(Default)]
pub struct HttpResponseModuleLoader;

impl ModuleLoader for HttpResponseModuleLoader {
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
        registry.with_stage::<ferron_http::HttpContext, _>(move || {
            Arc::new(HttpResponseStage::new(engine.clone()))
        })
    }
}
