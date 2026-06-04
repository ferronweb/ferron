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
            "alidns",
            "arvancloud",
            "autodns",
            "azuredns",
            "baiducloud",
            "bluecatv2",
            "bunny",
            "cloudflare",
            "cloudns",
            "constellix",
            "cpanel",
            "ddnss",
            "desec",
            "digitalocean",
            "dnsimple",
            "dnsmadeeasy",
            "domeneshop",
            "dreamhost",
            "duckdns",
            "dynu",
            "easydns",
            "edgedns",
            "exoscale",
            "freemyip",
            "gandiv5",
            "gcore",
            "glesys",
            "godaddy",
            "googlecloud",
            "hetzner",
            "hostingde",
            "hostinger",
            "huaweicloud",
            "hurricane",
            "ibmcloud",
            "infoblox",
            "infomaniak",
            "ionos",
            "ipv64",
            "inwx",
            "joker",
            "lightsail",
            "linode",
            "luadns",
            "mythicbeasts",
            "namecheap",
            "namedotcom",
            "namesilo",
            "netcup",
            "netlify",
            "nifcloud",
            "ns1",
            "oraclecloud",
            "ovh",
            "plesk",
            "porkbun",
            "rfc2136",
            "route53",
            "safedns",
            "scaleway",
            "spaceship",
            "tencentcloud",
            "transip",
            "ultradns",
            "vercel",
            "volcengine",
            "vultr",
            "websupport",
            "yandexcloud",
        ];

        for &name in &providers {
            registry.insert(
                config_validator_scoped_key!("dns", name),
                Box::new(validator::DnsStalwartConfigurationValidator),
            );
        }
    }
}
