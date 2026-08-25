//! Module loader for HTTP abuse protection.

use std::collections::HashMap;
use std::sync::Arc;

use ferron_core::loader::ModuleLoader;
use ferron_core::registry::RegistryBuilder;
use ferron_http::abuse;

use crate::registry::AbuseRegistry;
use crate::stage::AbuseProtectionStage;
use crate::validator::AbuseProtectionValidator;

/// Module loader for HTTP abuse protection.
#[derive(Default)]
pub struct HttpAbuseProtectionModuleLoader;

impl ModuleLoader for HttpAbuseProtectionModuleLoader {
    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn ferron_core::config::validator::ConfigurationValidator>>,
    ) {
        registry.push(Box::new(AbuseProtectionValidator));
    }

    fn register_directives(&mut self, registry: &mut ferron_core::directives::DirectiveRegistry) {
        use ferron_core::directives::{Directive, DirectiveSubblock};
        registry
            .register(
                Directive {
                    name: "abuse_protection",
                    usage: "abuse_protection [bool] | abuse_protection { ... }",
                    description: "This directive enables or configures abuse protection with threshold and ban settings.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: Some(DirectiveSubblock::custom("http_abuse_protection")),
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "abuse_event",
                    usage: "abuse_event <name>",
                    description: "This directive registers a custom abuse event name for honeypot or scanner detection.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::default(),
            )
            .register(
                Directive {
                    name: "ban_duration",
                    usage: "ban_duration <duration>",
                    description: "This directive sets the ban duration for abusive IP addresses.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_abuse_protection"),
            )
            .register(
                Directive {
                    name: "rate_limit_threshold",
                    usage: "rate_limit_threshold { events ...; window ... }",
                    description: "This directive configures the rate limit threshold with events and window parameters.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_abuse_protection"),
            )
            .register(
                Directive {
                    name: "brute_force_threshold",
                    usage: "brute_force_threshold { events ...; window ... }",
                    description: "This directive configures the brute force threshold with events and window parameters.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_abuse_protection"),
            )
            .register(
                Directive {
                    name: "custom_threshold",
                    usage: "custom_threshold { events ...; window ... }",
                    description: "This directive configures a custom threshold with events and window parameters.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_abuse_protection"),
            )
            .register(
                Directive {
                    name: "error_rate_threshold",
                    usage: "error_rate_threshold { events ...; window ...; status_codes ... }",
                    description: "This directive configures the error rate threshold with events, window, and status codes.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_abuse_protection"),
            )
            .register(
                Directive {
                    name: "allowlist",
                    usage: "allowlist <ip-or-cidr>...",
                    description: "This directive specifies IP addresses or CIDR ranges exempt from abuse protection.",
                    applicable_protocols: Some(&["http"]),
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("http_abuse_protection"),
            );
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
            .push(Box::new(AbuseProtectionValidator));
    }

    fn register_stages(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        let abuse_registry = Arc::new(AbuseRegistry::new());

        // Share the registry globally so rate limit and basic auth modules
        // can emit abuse events without depending on this crate.
        let _ = abuse::set_global_abuse_recorder(
            abuse_registry.clone() as Arc<dyn abuse::AbuseRecorder>
        );

        registry.with_stage::<ferron_http::HttpContext, _>(move || {
            Arc::new(AbuseProtectionStage::new(abuse_registry.clone()))
        })
    }
}
