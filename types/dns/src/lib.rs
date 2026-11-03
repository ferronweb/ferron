//! DNS provider types for certificate management.
//!
//! This crate defines the types used by DNS-based certificate providers
//! (e.g. ACME DNS-01 challenges). Modules implement [`DnsClient`] to
//! interact with a specific DNS provider API.
//!
//! # For module authors
//!
//! To implement a DNS provider, create a struct that implements
//! [`DnsClient`] and a [`Provider<DnsContext>`](ferron_core::providers::Provider)
//! that initializes the client from configuration.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use ferron_core::config::ServerConfigurationBlock;

/// A DNS record that can be created or updated via a [`DnsClient`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnsRecord {
    /// Record name (e.g. `"example.com"` or `"*.example.com"`).
    pub name: String,
    /// Record type (e.g. `"A"`, `"AAAA"`, `"CNAME"`, `"TXT"`, `"MX"`).
    pub record_type: DnsRecordType,
    /// Record value (e.g. `"1.2.3.4"` for A records).
    pub value: String,
    /// Time-to-live in seconds. Must be >= the client's [`DnsClient::minimum_ttl`].
    pub ttl: u32,
}

/// Well-known DNS record types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DnsRecordType {
    /// IPv4 address record.
    A,
    /// IPv6 address record.
    AAAA,
    /// Canonical name record.
    CNAME,
    /// Text record (used for ACME DNS-01 challenges).
    TXT,
    /// Mail exchange record.
    MX,
    /// Name server record.
    NS,
    /// Service locator record.
    SRV,
    /// Certification authority authorization record.
    CAA,
    /// DANE TLS association record.
    TLSA,
    /// HTTPS binding record.
    HTTPS,
}

impl fmt::Display for DnsRecordType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DnsRecordType::A => write!(f, "A"),
            DnsRecordType::AAAA => write!(f, "AAAA"),
            DnsRecordType::CNAME => write!(f, "CNAME"),
            DnsRecordType::TXT => write!(f, "TXT"),
            DnsRecordType::MX => write!(f, "MX"),
            DnsRecordType::NS => write!(f, "NS"),
            DnsRecordType::SRV => write!(f, "SRV"),
            DnsRecordType::CAA => write!(f, "CAA"),
            DnsRecordType::TLSA => write!(f, "TLSA"),
            DnsRecordType::HTTPS => write!(f, "HTTPS"),
        }
    }
}

impl std::str::FromStr for DnsRecordType {
    type Err = DnsProviderError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().as_str() {
            "A" => Ok(DnsRecordType::A),
            "AAAA" => Ok(DnsRecordType::AAAA),
            "CNAME" => Ok(DnsRecordType::CNAME),
            "TXT" => Ok(DnsRecordType::TXT),
            "MX" => Ok(DnsRecordType::MX),
            "TLSA" => Ok(DnsRecordType::TLSA),
            "NS" => Ok(DnsRecordType::NS),
            "HTTPS" => Ok(DnsRecordType::HTTPS),
            "SRV" => Ok(DnsRecordType::SRV),
            "CAA" => Ok(DnsRecordType::CAA),
            _ => Err(DnsProviderError(format!("unknown DNS record type: {s}"))),
        }
    }
}

/// Error type for DNS provider operations.
#[derive(Debug)]
pub struct DnsProviderError(String);

impl DnsProviderError {
    /// Create a new DNS provider error with the given message.
    pub fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}

impl fmt::Display for DnsProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DNS provider error: {}", self.0)
    }
}

impl std::error::Error for DnsProviderError {}

/// Async trait for DNS provider clients.
///
/// Implementations are created by [`Provider<DnsContext>`](ferron_core::providers::Provider) implementations
/// and stored in [`DnsContext::client`].
#[async_trait]
pub trait DnsClient: Send + Sync {
    /// Returns the minimum TTL (in seconds) allowed by this DNS provider.
    ///
    /// Any [`DnsRecord::ttl`] below this value will be rejected.
    fn minimum_ttl(&self) -> u32;

    /// Creates or updates a DNS record.
    async fn update_record(&self, record: &DnsRecord) -> Result<(), DnsProviderError>;

    /// Deletes all records matching the given name and type.
    async fn delete_record(
        &self,
        name: &str,
        record_type: DnsRecordType,
    ) -> Result<(), DnsProviderError>;
}

/// Context passed to DNS [`Provider`](ferron_core::providers::Provider) implementations.
///
/// A provider reads [`DnsContext::config`] to obtain API credentials
/// (token, zone ID, endpoint, etc.) and sets [`DnsContext::client`]
/// to an initialized [`DnsClient`].
///
/// # Example
///
/// ```ignore
/// pub struct CloudflareDnsProvider { /* ... */ }
///
/// impl Provider<DnsContext<'_>> for CloudflareDnsProvider {
///     fn name(&self) -> &str { "cloudflare" }
///
///     fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
///         let client = CloudflareClient::from_config(ctx.config)?;
///         ctx.client = Some(Arc::new(client));
///         Ok(())
///     }
/// }
/// ```
pub struct DnsContext<'a> {
    /// Configuration block from the server config (e.g. `dns { ... }`).
    pub config: &'a ServerConfigurationBlock,
    /// The initialized DNS client, set by the provider during [`execute`](ferron_core::providers::Provider::execute).
    pub client: Option<Arc<dyn DnsClient>>,
}

/// Separates subdomain from domain name.
pub async fn separate_subdomain_from_domain_name(domain_name: &str) -> (String, String) {
    let parts: Vec<&str> = domain_name
        .strip_suffix(".")
        .unwrap_or(domain_name)
        .split('.')
        .collect();
    let Some(resolver) = system_resolver() else {
        return ("".to_string(), parts.join("."));
    };

    for parts_index in 0..parts.len() {
        if resolver
            .soa_lookup(format!("{}.", parts[parts_index..].join(".")))
            .await
            .is_ok()
        {
            // SOA record found
            let subdomain = parts[..parts_index].join(".");
            let domain = parts[parts_index..].join(".");
            return (subdomain, domain);
        }
    }

    ("".to_string(), parts.join("."))
}

/// Builds a resolver from the system configuration, or the default one.
fn system_resolver() -> Option<hickory_resolver::TokioResolver> {
    hickory_resolver::Resolver::builder_tokio()
        .unwrap_or(hickory_resolver::Resolver::builder_with_config(
            hickory_resolver::config::ResolverConfig::default(),
            hickory_resolver::net::runtime::TokioRuntimeProvider::default(),
        ))
        .build()
        .ok()
}

/// Checks a TXT record on every authoritative nameserver of its zone.
///
/// Confirms that a record created through a [`DnsClient`] (for example an
/// ACME DNS-01 challenge) is visible to the servers that validators query.
/// Queries go from this host, so they can't see anycast sites elsewhere.
pub struct AuthoritativeTxtCheck {
    /// Fully qualified record name.
    name: String,
    /// For each nameserver, one uncached, non-recursive resolver per address.
    nameservers: Vec<Vec<hickory_resolver::TokioResolver>>,
}

impl AuthoritativeTxtCheck {
    /// Finds the authoritative nameservers of the zone that contains `name`.
    ///
    /// Returns `None` when the zone, any of its nameservers, or their addresses
    /// can't be resolved, for example for private zones the system resolver
    /// can't see, so callers never check only some of the nameservers.
    pub async fn new(name: &str) -> Option<Self> {
        use hickory_resolver::config::{NameServerConfig, ResolverConfig, ResolverOpts};
        use hickory_resolver::proto::rr::RData;

        let name = format!("{}.", name.trim_end_matches('.'));
        let resolver = system_resolver()?;
        let (_, zone) = separate_subdomain_from_domain_name(&name).await;
        let ns_lookup = resolver.ns_lookup(format!("{zone}.")).await.ok()?;

        let mut options = ResolverOpts::default();
        options.cache_size = 0;
        options.recursion_desired = false;
        options.timeout = std::time::Duration::from_secs(2);
        options.attempts = 1;

        let mut nameservers = Vec::new();
        for record in ns_lookup.answers() {
            let RData::NS(ns) = &record.data else {
                continue;
            };
            let addresses = resolver.lookup_ip(ns.0.clone()).await.ok()?;
            let resolvers = addresses
                .iter()
                .map(|address| {
                    hickory_resolver::Resolver::builder_with_config(
                        ResolverConfig::from_parts(
                            None,
                            Vec::new(),
                            vec![NameServerConfig::udp_and_tcp(address)],
                        ),
                        hickory_resolver::net::runtime::TokioRuntimeProvider::default(),
                    )
                    .with_options(options.clone())
                    .build()
                    .ok()
                })
                .collect::<Option<Vec<_>>>()?;
            if resolvers.is_empty() {
                return None;
            }
            nameservers.push(resolvers);
        }

        (!nameservers.is_empty()).then_some(Self { name, nameservers })
    }

    /// Whether every nameserver serves a TXT record for the name with exactly `value`.
    ///
    /// Each address of a nameserver that answers must serve the value, and at
    /// least one must answer; unreachable addresses (for example IPv6 ones on
    /// an IPv4-only host) are skipped.
    pub async fn is_served(&self, value: &str) -> bool {
        use hickory_resolver::proto::rr::RData;

        for addresses in &self.nameservers {
            let mut answered = false;
            for address in addresses {
                match address.txt_lookup(self.name.as_str()).await {
                    Ok(lookup) => {
                        let served = lookup.answers().iter().any(|record| {
                            matches!(&record.data, RData::TXT(txt)
                                if txt.txt_data.iter().flat_map(|part| part.iter()).copied().eq(value.bytes()))
                        });
                        if !served {
                            return false;
                        }
                        answered = true;
                    }
                    Err(error) if error.is_no_records_found() => return false,
                    Err(_) => {}
                }
            }
            if !answered {
                return false;
            }
        }
        true
    }
}
