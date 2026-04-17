mod bunny;
mod cloudflare;
mod desec;
mod digitalocean;
mod dnsimple;
mod googlecloud;
mod ovh;
mod porkbun;
mod rfc2136;
mod route53;
mod spaceship;

use std::sync::Arc;

use ferron_dns::DnsContext;

pub fn register_providers(
    registry: ferron_core::registry::RegistryBuilder,
) -> ferron_core::registry::RegistryBuilder {
    registry
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(bunny::BunnyDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(cloudflare::CloudflareDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(desec::DesecDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(digitalocean::DigitalOceanDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(dnsimple::DnsimpleDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(googlecloud::GoogleCloudDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(ovh::OvhDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(porkbun::PorkbunDnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(route53::Route53DnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(rfc2136::Rfc2136DnsProvider))
        .with_provider::<DnsContext<'static>, _>(|| Arc::new(spaceship::SpaceshipDnsProvider))
}
