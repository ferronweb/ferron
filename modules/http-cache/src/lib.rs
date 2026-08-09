//! HTTP response cache with LSCache-compatible response header controls.

#![cfg_attr(feature = "fuzz", allow(private_interfaces))]

mod config;
#[cfg(feature = "fuzz")]
pub mod lscache;
#[cfg(not(feature = "fuzz"))]
mod lscache;
#[cfg(feature = "fuzz")]
pub mod policy;
#[cfg(not(feature = "fuzz"))]
mod policy;
mod stage;
#[cfg(feature = "fuzz")]
pub mod store;
#[cfg(not(feature = "fuzz"))]
mod store;
mod validator;

use std::sync::{Arc, OnceLock};

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::loader::ModuleLoader;
use ferron_core::registry::RegistryBuilder;
use ferron_http::HttpContext;

pub use stage::HttpCacheStage;
pub use validator::HttpCacheConfigurationValidator;

pub static SECONDARY_RUNTIME: OnceLock<tokio::runtime::Handle> = OnceLock::new();

/// Module loader for the HTTP cache module.
#[derive(Default)]
pub struct HttpCacheModuleLoader {
    cache: Option<Arc<HttpCacheModule>>,
}

impl ModuleLoader for HttpCacheModuleLoader {
    #[inline]
    fn register_directives(&mut self, registry: &mut ferron_core::directives::DirectiveRegistry) {
        register_cache_policy_directives(registry);
        register_cache_litespeed_directives(registry);
        register_cache_stale_directives(registry);
        register_cache_purge_directives(registry);
    }

    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn ConfigurationValidator>>,
    ) {
        registry.push(Box::new(HttpCacheConfigurationValidator));
    }

    #[inline]
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
            .push(Box::new(HttpCacheConfigurationValidator));
    }

    #[inline]
    fn register_stages(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        let stage = Arc::new(HttpCacheStage::new());
        registry.with_stage::<HttpContext, _>(move || stage.clone())
    }

    #[inline]
    fn register_modules(
        &mut self,
        _registry: Arc<ferron_core::registry::Registry>,
        modules: &mut Vec<Arc<dyn ferron_core::Module>>,
        _config: Arc<ferron_core::config::ServerConfiguration>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if self.cache.is_none() {
            let module = Arc::new(HttpCacheModule);
            modules.push(module.clone());
            self.cache = Some(module);
        }
        Ok(())
    }
}

fn register_cache_policy_directives(registry: &mut ferron_core::directives::DirectiveRegistry) {
    use ferron_core::directives::{Directive, DirectiveSubblock};
    registry
        .register(
            Directive {
                name: "cache",
                usage: "cache [bool] | cache { ... }",
                description: "This directive enables or configures HTTP response caching with zone, policy, and purge settings.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: Some(DirectiveSubblock::custom("http_cache")),
            },
            DirectiveSubblock::default(),
        )
        .register(
            Directive {
                name: "max_entries",
                usage: "max_entries <count>",
                description: "This directive sets the maximum number of cache entries.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("http_cache"),
        )
        .register(
            Directive {
                name: "max_response_size",
                usage: "max_response_size <bytes>",
                description: "This directive sets the maximum response body size in bytes for caching.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("http_cache"),
        )
        .register(
            Directive {
                name: "zone",
                usage: "zone <name> | zone <name> { ... }",
                description: "This directive assigns a host to a named cache zone or defines a named zone at global scope.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("http_cache"),
        )
        .register(
            Directive {
                name: "vary",
                usage: "vary <header>...",
                description: "This directive adds request header names to include in the cache key.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("http_cache"),
        )
        .register(
            Directive {
                name: "vary_cookies",
                usage: "vary_cookies <cookie>...",
                description: "This directive adds cookie names to include in the cache key.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("http_cache"),
        )
        .register(
            Directive {
                name: "ignore",
                usage: "ignore <header>...",
                description: "This directive strips response headers from the cached representation.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("http_cache"),
        );
}

fn register_cache_litespeed_directives(registry: &mut ferron_core::directives::DirectiveRegistry) {
    use ferron_core::directives::{Directive, DirectiveSubblock};
    registry
        .register(
            Directive {
                name: "litespeed_override_cache_control",
                usage: "litespeed_override_cache_control [bool]",
                description: "This directive allows X-LiteSpeed-Cache-Control to override standard Cache-Control headers.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("http_cache"),
        )
        .register(
            Directive {
                name: "emit_litespeed_headers",
                usage: "emit_litespeed_headers [bool]",
                description: "This directive enables X-LiteSpeed-Cache response headers on cached responses.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("http_cache"),
        )
        .register(
            Directive {
                name: "ignore_request_cache_control",
                usage: "ignore_request_cache_control [bool]",
                description: "This directive ignores client Cache-Control and Pragma headers.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("http_cache"),
        );
}

fn register_cache_stale_directives(registry: &mut ferron_core::directives::DirectiveRegistry) {
    use ferron_core::directives::{Directive, DirectiveSubblock};
    registry
        .register(
            Directive {
                name: "enable_stale_while_revalidate",
                usage: "enable_stale_while_revalidate [bool]",
                description: "This directive enables serving stale responses while revalidating in the background.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("http_cache"),
        )
        .register(
            Directive {
                name: "enable_stale_if_error",
                usage: "enable_stale_if_error [bool]",
                description: "This directive enables serving stale responses when the backend is unavailable.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("http_cache"),
        )
        .register(
            Directive {
                name: "coalesce_timeout",
                usage: "coalesce_timeout <seconds>",
                description: "This directive sets how long a coalesced request waits for the in-flight fetch leader before fetching upstream itself.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("http_cache"),
        );
}

fn register_cache_purge_directives(registry: &mut ferron_core::directives::DirectiveRegistry) {
    use ferron_core::directives::{Directive, DirectiveSubblock};
    registry
        .register(
            Directive {
                name: "purge_method",
                usage: "purge_method [bool]",
                description: "This directive enables PURGE HTTP method for cache invalidation.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("http_cache"),
        )
        .register(
            Directive {
                name: "purge_allowed_ips",
                usage: "purge_allowed_ips <ip-or-cidr>...",
                description: "This directive restricts cache PURGE requests to specific IPs or CIDR ranges.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("http_cache"),
        )
        .register(
            Directive {
                name: "purge_propagation",
                usage: "purge_propagation [bool] | purge_propagation { ... }",
                description: "This directive enables multi-instance purge propagation with control-plane settings.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("http_cache"),
        )
        .register(
            Directive {
                name: "control_plane_url",
                usage: "control_plane_url <url>",
                description: "This directive sets the control-plane URL for purge propagation.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("http_cache"),
        )
        .register(
            Directive {
                name: "shared_secret",
                usage: "shared_secret <value>",
                description: "This directive sets the shared secret for inter-node purge authentication.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("http_cache"),
        )
        .register(
            Directive {
                name: "node_id",
                usage: "node_id <id>",
                description: "This directive sets the node identifier for purge propagation.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("http_cache"),
        );
}

#[derive(Default)]
pub struct HttpCacheModule;

impl ferron_core::Module for HttpCacheModule {
    #[inline]
    fn name(&self) -> &str {
        "http-cache"
    }

    #[inline]
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    #[inline]
    fn start(
        &self,
        runtime: &mut ferron_core::runtime::Runtime,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let _ = SECONDARY_RUNTIME
            .set(runtime.block_on(async move { tokio::runtime::Handle::current() }));
        Ok(())
    }
}
