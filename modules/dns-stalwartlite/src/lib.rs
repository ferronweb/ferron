mod client;
mod providers;
pub(crate) mod validator;

use ferron_core::loader::ModuleLoader;

pub struct StalwartDnsModuleLoader;

impl ModuleLoader for StalwartDnsModuleLoader {
    fn register_providers(
        &mut self,
        registry: ferron_core::registry::RegistryBuilder,
    ) -> ferron_core::registry::RegistryBuilder {
        providers::register_providers(registry)
    }

    fn register_scoped_configuration_validators(
        &mut self,
        registry: &mut std::collections::HashMap<
            ferron_core::config::validator::ConfigurationValidatorScopedKey,
            Box<dyn ferron_core::config::validator::ConfigurationValidator>,
        >,
    ) {
        use ferron_core::config_validator_scoped_key;

        let providers = [
            "bunny",
            "cloudflare",
            "desec",
            "digitalocean",
            "dnsimple",
            "googlecloud",
            "ovh",
            "porkbun",
            "rfc2136",
            "route53",
            "spaceship",
        ];

        for &name in &providers {
            registry.insert(
                config_validator_scoped_key!("dns", name),
                Box::new(validator::DnsStalwartConfigurationValidator),
            );
        }
    }
}
