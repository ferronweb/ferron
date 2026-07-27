use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use cegla_fcgi::client::SendRequest;
use ferron_http::{HttpContext, HttpFileContext};
use http_body_util::combinators::UnsyncBoxBody;
use parking_lot::RwLock;

use crate::stages::{FcgiFileStage, FcgiPassStage};

mod client;
mod config;
mod stages;
mod util;
mod validator;

const DEFAULT_CONCURRENT_CONNECTIONS: usize = 16384;

static GLOBAL_CONCURRENT_CONNECTIONS: AtomicUsize =
    AtomicUsize::new(DEFAULT_CONCURRENT_CONNECTIONS);

/// Body type used for FastCGI requests.
pub type ProxyBody = UnsyncBoxBody<bytes::Bytes, std::io::Error>;

/// Connection pool item containing the HTTP client.
pub type ConnpoolItem = connpool::Item<String, SendRequest<ProxyBody>>;

pub struct FcgiModuleLoader {
    client: RwLock<client::FcgiClient>,
}

impl Default for FcgiModuleLoader {
    fn default() -> Self {
        Self {
            client: RwLock::new(client::FcgiClient::new(DEFAULT_CONCURRENT_CONNECTIONS)),
        }
    }
}

impl ferron_core::loader::ModuleLoader for FcgiModuleLoader {
    fn register_directives(&mut self, registry: &mut ferron_core::directives::DirectiveRegistry) {
        use ferron_core::directives::{Directive, DirectiveSubblock};
        registry
            .register(
                Directive {
                    name: "fcgi",
                    usage: "fcgi [bool] | fcgi { ... }",
                    description: "This directive enables FastCGI proxying with backend, extension, environment, and connection settings.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: Some(DirectiveSubblock::custom("http_fcgi")),
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "fcgi_php",
                    usage: "fcgi_php <url> | fcgi_php false",
                    description: "This directive enables PHP-FPM support as a shorthand for fcgi with .php extension.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "fcgi_concurrent_conns",
                    usage: "fcgi_concurrent_conns <limit>",
                    description: "This directive sets the global limit for concurrent FastCGI connections.",
                    applicable_protocols: Some(&["http"]),
                    global_only: true,
                    subblock_link: None,
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "backend",
                    usage: "backend <url>",
                    description: "This directive specifies the FastCGI backend URL.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_fcgi"),
            )
            .register(
                Directive {
                    name: "extension",
                    usage: "extension <.ext>...",
                    description: "This directive registers file extensions as FastCGI script handlers.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_fcgi"),
            )
            .register(
                Directive {
                    name: "environment",
                    usage: "environment <name> <value>",
                    description: "This directive sets a FastCGI environment variable.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_fcgi"),
            )
            .register(
                Directive {
                    name: "pass",
                    usage: "pass [bool]",
                    description: "This directive passes files that match extensions to the backend for processing.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_fcgi"),
            )
            .register(
                Directive {
                    name: "keepalive",
                    usage: "keepalive [bool]",
                    description: "This directive enables keepalive connections to the FastCGI backend.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_fcgi"),
            )
            .register(
                Directive {
                    name: "limit",
                    usage: "limit <count>",
                    description: "This directive sets the per-backend connection pool limit for FastCGI.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_fcgi"),
            );
    }

    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn ferron_core::config::validator::ConfigurationValidator>>,
    ) {
        registry.push(Box::new(validator::FcgiConfigurationValidator));
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
            .push(Box::new(validator::FcgiConfigurationValidator));
    }

    fn register_stages(
        &mut self,
        registry: ferron_core::registry::RegistryBuilder,
    ) -> ferron_core::registry::RegistryBuilder {
        let client = Arc::new(self.client.read().clone());
        let client2 = client.clone();
        registry
            .with_stage::<HttpContext, _>(move || Arc::new(FcgiPassStage::new(client.clone())))
            .with_stage::<HttpFileContext, _>(move || Arc::new(FcgiFileStage::new(client2.clone())))
    }

    fn register_modules(
        &mut self,
        _registry: Arc<ferron_core::registry::Registry>,
        _modules: &mut Vec<Arc<dyn ferron_core::Module>>,
        config: Arc<ferron_core::config::ServerConfiguration>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(val) = config
            .global_config
            .directives
            .get("fcgi_concurrent_conns")
            .and_then(|entries| entries.first())
            .and_then(|e| e.args.first())
            .and_then(|v: &ferron_core::config::ServerConfigurationValue| v.as_number())
        {
            if val > 0 {
                let new_limit = val as usize;
                let old_limit =
                    GLOBAL_CONCURRENT_CONNECTIONS.load(std::sync::atomic::Ordering::Relaxed);
                GLOBAL_CONCURRENT_CONNECTIONS
                    .store(new_limit, std::sync::atomic::Ordering::Relaxed);

                if old_limit != new_limit {
                    self.client.write().update_global_limit(new_limit);
                }
            }
        }

        Ok(())
    }
}
