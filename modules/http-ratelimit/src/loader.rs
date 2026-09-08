//! Module loader implementation for HTTP rate limiting.

use std::collections::HashMap;
use std::sync::Arc;

use ferron_core::loader::ModuleLoader;
use ferron_core::registry::RegistryBuilder;

use crate::backends::runtime::capture_secondary_handle;
use crate::stage::{RateLimitEngine, RateLimitStage};
use crate::validator::RateLimitValidator;

#[derive(Default)]
pub struct HttpRateLimitModuleLoader;

/// Runtime module that captures the secondary Tokio handle for Redis I/O.
pub struct HttpRateLimitModule;

impl ferron_core::Module for HttpRateLimitModule {
    #[inline]
    fn name(&self) -> &str {
        "http-ratelimit"
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
        capture_secondary_handle(runtime);
        Ok(())
    }
}

impl ModuleLoader for HttpRateLimitModuleLoader {
    fn register_directives(&mut self, registry: &mut ferron_core::directives::DirectiveRegistry) {
        register_rate_limit_directives(registry);
        register_backend_directives(registry);
    }

    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn ferron_core::config::validator::ConfigurationValidator>>,
    ) {
        registry.push(Box::new(RateLimitValidator));
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
            .push(Box::new(RateLimitValidator));
    }

    fn register_stages(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        let engine = Arc::new(RateLimitEngine::new());
        registry.with_stage::<ferron_http::HttpContext, _>(move || {
            Arc::new(RateLimitStage::new(engine.clone()))
        })
    }

    fn register_modules(
        &mut self,
        _registry: Arc<ferron_core::registry::Registry>,
        modules: &mut Vec<Arc<dyn ferron_core::Module>>,
        _config: Arc<ferron_core::config::ServerConfiguration>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        modules.push(Arc::new(HttpRateLimitModule));
        Ok(())
    }
}

/// Register the `rate_limit` directive and its subdirectives.
fn register_rate_limit_directives(registry: &mut ferron_core::directives::DirectiveRegistry) {
    use ferron_core::directives::{Directive, DirectiveSubblock};
    registry
        .register(
            Directive {
                name: "rate_limit",
                usage: "rate_limit { ... }",
                description: "This directive configures rate limiting with rate, burst, key, deny status, bucket TTL, max buckets, throttle, and zone settings.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: Some(DirectiveSubblock::custom("http_rate_limit")),
            },
            DirectiveSubblock::default(),
        )
        .register(
            Directive {
                name: "rate",
                usage: "rate <count>",
                description: "This directive sets the rate limit in requests per second.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("http_rate_limit"),
        )
        .register(
            Directive {
                name: "burst",
                usage: "burst <count>",
                description: "This directive sets the burst size for the rate limiter.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("http_rate_limit"),
        )
        .register(
            Directive {
                name: "key",
                usage: "key <type>",
                description: "This directive selects the rate limit key type: remote_address, uri, or request.header.<name>.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("http_rate_limit"),
        )
        .register(
            Directive {
                name: "deny_status",
                usage: "deny_status <code>",
                description: "This directive sets the HTTP status code returned when rate limit is exceeded.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("http_rate_limit"),
        )
        .register(
            Directive {
                name: "bucket_ttl",
                usage: "bucket_ttl <duration>",
                description: "This directive sets the TTL for rate limit buckets.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("http_rate_limit"),
        )
        .register(
            Directive {
                name: "max_buckets",
                usage: "max_buckets <count>",
                description: "This directive sets the maximum number of rate limit buckets.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("http_rate_limit"),
        )
        .register(
            Directive {
                name: "throttle",
                usage: "throttle [bool]",
                description: "This directive enables throttling instead of denying requests when rate limit is exceeded.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("http_rate_limit"),
        )
        .register(
            Directive {
                name: "zone",
                usage: "zone <name>",
                description: "This directive assigns or defines a named rate limit zone.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("http_rate_limit"),
        );
}

/// Register the `backend` directive and its subdirectives.
fn register_backend_directives(registry: &mut ferron_core::directives::DirectiveRegistry) {
    use ferron_core::directives::{Directive, DirectiveSubblock};
    registry
        .register(
            Directive {
                name: "rate_limit_backend",
                usage: "rate_limit_backend { ... }",
                description: "This directive configures the rate limit storage backend (memory or Redis/Valkey). Inherits from global scope when not set.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: Some(DirectiveSubblock::custom("http_ratelimit_backend")),
            },
            DirectiveSubblock::default(),
        )
        .register(
            Directive {
                name: "type",
                usage: "type <memory|redis>",
                description: "This directive selects the rate limit backend type.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("http_ratelimit_backend"),
        )
        .register(
            Directive {
                name: "url",
                usage: "url <redis-url>",
                description: "This directive sets the Redis/Valkey URL (required for redis backends).",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("http_ratelimit_backend"),
        )
        .register(
            Directive {
                name: "key_prefix",
                usage: "key_prefix <prefix>",
                description: "This directive sets the prefix for Redis rate limit keys.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("http_ratelimit_backend"),
        )
        .register(
            Directive {
                name: "timeout",
                usage: "timeout <duration>",
                description: "This directive sets the per-request timeout for Redis operations.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("http_ratelimit_backend"),
        )
        .register(
            Directive {
                name: "fail_open",
                usage: "fail_open [bool]",
                description: "This directive controls whether Redis errors allow (true, default) or deny (false) requests.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::custom("http_ratelimit_backend"),
        );
}
