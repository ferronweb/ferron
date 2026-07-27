mod config;
mod stage;
mod validator;

pub use stage::HttpTraceIdStage;
pub use validator::HttpTraceIdConfigurationValidator;

use std::sync::Arc;

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::loader::ModuleLoader;
use ferron_core::registry::RegistryBuilder;
use ferron_http::HttpContext;

#[derive(Default)]
pub struct HttpTraceIdModuleLoader;

impl ModuleLoader for HttpTraceIdModuleLoader {
    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn ConfigurationValidator>>,
    ) {
        registry.push(Box::new(HttpTraceIdConfigurationValidator));
    }

    fn register_directives(&mut self, registry: &mut ferron_core::directives::DirectiveRegistry) {
        use ferron_core::directives::{Directive, DirectiveSubblock};
        registry
            .register(
                Directive {
                    name: "trace_id_header",
                    usage: "trace_id_header [bool] | trace_id_header { ... }",
                    description: "This directive enables or configures a custom trace ID header with reflection and name settings.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: Some(DirectiveSubblock::custom("http_trace_id_header")),
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "reflect_request",
                    usage: "reflect_request [bool]",
                    description: "This directive enables reflecting the trace ID back in a response header.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_trace_id_header"),
            )
            .register(
                Directive {
                    name: "header_name",
                    usage: "header_name <name>",
                    description: "This directive sets the custom header name for trace ID propagation.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_trace_id_header"),
            );
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
            .push(Box::new(HttpTraceIdConfigurationValidator));
    }

    fn register_stages(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        registry.with_stage::<HttpContext, _>(|| Arc::new(HttpTraceIdStage::new()))
    }
}
