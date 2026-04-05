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
    A,
    AAAA,
    CNAME,
    TXT,
    MX,
    NS,
    SRV,
    CAA,
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
            "NS" => Ok(DnsRecordType::NS),
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
/// Implementations are created by [`Provider<DnsContext>`] implementations
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
    async fn delete_record(&self, name: &str, record_type: &str) -> Result<(), DnsProviderError>;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dns_record_type_display() {
        assert_eq!(DnsRecordType::A.to_string(), "A");
        assert_eq!(DnsRecordType::TXT.to_string(), "TXT");
        assert_eq!(DnsRecordType::CNAME.to_string(), "CNAME");
    }

    #[test]
    fn dns_record_type_from_str() {
        assert_eq!("a".parse::<DnsRecordType>().unwrap(), DnsRecordType::A);
        assert_eq!("AAAA".parse::<DnsRecordType>().unwrap(), DnsRecordType::AAAA);
        assert!("INVALID".parse::<DnsRecordType>().is_err());
    }

    #[test]
    fn dns_provider_error() {
        let err = DnsProviderError::new("something went wrong");
        assert_eq!(err.to_string(), "DNS provider error: something went wrong");
    }

    #[test]
    fn dns_record_roundtrip() {
        let record = DnsRecord {
            name: "example.com".into(),
            record_type: DnsRecordType::A,
            value: "1.2.3.4".into(),
            ttl: 300,
        };
        assert_eq!(record.name, "example.com");
        assert_eq!(record.record_type, DnsRecordType::A);
        assert_eq!(record.value, "1.2.3.4");
        assert_eq!(record.ttl, 300);
    }
}
