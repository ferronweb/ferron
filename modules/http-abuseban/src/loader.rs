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
