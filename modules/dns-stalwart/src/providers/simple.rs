//! Simple DNS providers that require only one credential.

crate::dns_provider!(
    ArvanCloudDnsProvider,
    "arvancloud",
    "api_key",
    "API key",
    new_arvancloud,
    60
);
crate::dns_provider!(
    BunnyDnsProvider,
    "bunny",
    "api_key",
    "API key",
    new_bunny,
    60
);
crate::dns_provider!(
    CloudflareDnsProvider,
    "cloudflare",
    "api_key",
    "API key",
    new_cloudflare,
    60
);
crate::dns_provider!(DDNSSProvider, "ddnss", "key", "key", new_ddnss, 60);
crate::dns_provider!(
    DesecDnsProvider,
    "desec",
    "auth_token",
    "auth token",
    new_desec,
    60
);
crate::dns_provider!(
    DigitalOceanDnsProvider,
    "digitalocean",
    "auth_token",
    "auth token",
    new_digitalocean,
    60
);
crate::dns_provider!(
    DreamHostDnsProvider,
    "dreamhost",
    "api_key",
    "API key",
    new_dreamhost,
    60
);
crate::dns_provider!(
    DuckDNSProvider,
    "duckdns",
    "token",
    "token",
    new_duckdns,
    60
);
crate::dns_provider!(DynuDnsProvider, "dynu", "api_key", "API key", new_dynu, 60);
crate::dns_provider!(
    FreeMyIpDnsProvider,
    "freemyip",
    "token",
    "token",
    new_freemyip,
    60
);
crate::dns_provider!(
    GandiV5DnsProvider,
    "gandiv5",
    "personal_access_token",
    "personal access token",
    new_gandiv5,
    60
);
crate::dns_provider!(
    GcoreDnsProvider,
    "gcore",
    "api_token",
    "API token",
    new_gcore,
    60
);
crate::dns_provider!(
    HetznerDnsProvider,
    "hetzner",
    "api_token",
    "API token",
    new_hetzner,
    60
);
crate::dns_provider!(
    HostingDeDnsProvider,
    "hostingde",
    "api_key",
    "API key",
    new_hostingde,
    60
);
crate::dns_provider!(
    HostingerDnsProvider,
    "hostinger",
    "api_token",
    "API token",
    new_hostinger,
    300
);
crate::dns_provider!(
    InfomaniakDnsProvider,
    "infomaniak",
    "api_token",
    "API token",
    new_infomaniak,
    60
);
crate::dns_provider!(
    IonosDnsProvider,
    "ionos",
    "api_key",
    "API key",
    new_ionos,
    60
);
crate::dns_provider!(
    IPv64DnsProvider,
    "ipv64",
    "api_key",
    "API key",
    new_ipv64,
    60
);
crate::dns_provider!(
    LinodeDnsProvider,
    "linode",
    "api_token",
    "API token",
    new_linode,
    60
);
crate::dns_provider!(
    NameSiloDnsProvider,
    "namesilo",
    "api_token",
    "API token",
    new_namesilo,
    60
);
crate::dns_provider!(
    NetlifyDnsProvider,
    "netlify",
    "access_token",
    "access token",
    new_netlify,
    60
);
crate::dns_provider!(NS1Provider, "ns1", "api_key", "API key", new_ns1, 0);
crate::dns_provider!(
    SafeDNSProvider,
    "safedns",
    "auth_token",
    "auth token",
    new_safedns,
    60
);
crate::dns_provider!(
    ScalewayDnsProvider,
    "scaleway",
    "api_token",
    "API token",
    new_scaleway,
    60
);
crate::dns_provider!(
    VultrDnsProvider,
    "vultr",
    "api_key",
    "API key",
    new_vultr,
    60
);
