mod dnsimple;
mod googlecloud;
mod ovh;
mod porkbun;
mod rfc2136;
mod route53;
mod simple;
mod spaceship;

pub(crate) mod util;

pub fn register_providers(
    registry: ferron_core::registry::RegistryBuilder,
) -> ferron_core::registry::RegistryBuilder {
    let registry = crate::register_providers!(registry,
        dnsimple => DnsimpleDnsProvider,
        googlecloud => GoogleCloudDnsProvider,
        ovh => OvhDnsProvider,
        porkbun => PorkbunDnsProvider,
        rfc2136 => Rfc2136DnsProvider,
        route53 => Route53DnsProvider,
        spaceship => SpaceshipDnsProvider,
    );
    crate::register_simple_providers!(
        registry,
        BunnyDnsProvider,
        CloudflareDnsProvider,
        DesecDnsProvider,
        DigitalOceanDnsProvider,
    )
}
