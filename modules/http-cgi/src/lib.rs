use std::sync::Arc;

use ferron_http::{HttpContext, HttpFileContext};

use crate::stages::{CgiInjectStage, CgiStage};

mod config;
mod stages;
mod util;
mod validator;

pub struct CgiModuleLoader;

impl ferron_core::loader::ModuleLoader for CgiModuleLoader {
    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn ferron_core::config::validator::ConfigurationValidator>>,
    ) {
        registry.push(Box::new(validator::CgiConfigurationValidator));
    }

    fn register_directives(&mut self, registry: &mut ferron_core::directives::DirectiveRegistry) {
        use ferron_core::directives::{Directive, DirectiveSubblock};
        registry
            .register(
                Directive {
                    name: "cgi",
                    usage: "cgi [bool] | cgi { ... }",
                    description: "This directive enables CGI execution and configures extension, interpreter, and environment settings.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: Some(DirectiveSubblock::custom("http_cgi")),
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "extension",
                    usage: "extension <.ext>",
                    description: "This directive registers a file extension as a CGI script handler.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_cgi"),
            )
            .register(
                Directive {
                    name: "interpreter",
                    usage: "interpreter <.ext> <command>...",
                    description: "This directive maps a file extension to a CGI interpreter command.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_cgi"),
            )
            .register(
                Directive {
                    name: "environment",
                    usage: "environment <name> <value>",
                    description: "This directive sets a CGI environment variable.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_cgi"),
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
            .push(Box::new(validator::CgiConfigurationValidator));
    }

    fn register_stages(
        &mut self,
        registry: ferron_core::registry::RegistryBuilder,
    ) -> ferron_core::registry::RegistryBuilder {
        registry
            .with_stage::<HttpFileContext, _>(|| Arc::new(CgiStage))
            .with_stage::<HttpContext, _>(|| Arc::new(CgiInjectStage))
    }
}
