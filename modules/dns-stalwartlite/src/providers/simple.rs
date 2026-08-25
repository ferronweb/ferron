//! Simple DNS providers that require only one credential.

crate::dns_provider!(BunnyDnsProvider, "bunny", "api_key", new_bunny, 60);
crate::dns_provider!(
    CloudflareDnsProvider,
    "cloudflare",
    "api_key",
    new_cloudflare,
    60
);
crate::dns_provider!(DesecDnsProvider, "desec", "auth_token", new_desec, 60);
crate::dns_provider!(
    DigitalOceanDnsProvider,
    "digitalocean",
    "auth_token",
    new_digitalocean,
    60
);
