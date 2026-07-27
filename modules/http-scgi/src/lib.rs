use std::sync::Arc;

use ferron_http::HttpContext;

use crate::stage::ScgiStage;

mod config;
mod stage;
mod util;
mod validator;

pub struct ScgiModuleLoader;

impl ferron_core::loader::ModuleLoader for ScgiModuleLoader {
    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn ferron_core::config::validator::ConfigurationValidator>>,
    ) {
        registry.push(Box::new(validator::ScgiConfigurationValidator));
    }

    fn register_directives(&mut self, registry: &mut ferron_core::directives::DirectiveRegistry) {
        use ferron_core::directives::{Directive, DirectiveSubblock};
        registry
            .register(
                Directive {
                    name: "scgi",
                    usage: "scgi <url> | scgi { ... }",
                    description: "This directive enables SCGI proxying with backend and environment settings.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: Some(DirectiveSubblock::custom("http_scgi")),
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "backend",
                    usage: "backend <url>",
                    description: "This directive specifies the SCGI backend URL (tcp:// or unix://).",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_scgi"),
            )
            .register(
                Directive {
                    name: "environment",
                    usage: "environment <name> <value>",
                    description: "This directive sets an SCGI environment variable.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_scgi"),
            );
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
            .push(Box::new(validator::ScgiConfigurationValidator));
    }

    fn register_stages(
        &mut self,
        registry: ferron_core::registry::RegistryBuilder,
    ) -> ferron_core::registry::RegistryBuilder {
        registry.with_stage::<HttpContext, _>(|| Arc::new(ScgiStage))
    }
}
