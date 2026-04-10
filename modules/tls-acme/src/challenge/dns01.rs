//! DNS-01 ACME challenge implementation.
//!
//! The DNS-01 challenge requires creating a TXT record at
//! `_acme-challenge.<domain>` with a specific value derived from the
//! key authorization.

use std::sync::Arc;

use ferron_dns::DnsClient;
use instant_acme::KeyAuthorization;

/// DNS-01 challenge helper.
///
/// Manages the creation and cleanup of ACME TXT records.
pub struct Dns01Helper {
    dns_client: Arc<dyn DnsClient>,
}

impl Dns01Helper {
    /// Creates a new `Dns01Helper` with the given DNS client.
    pub fn new(dns_client: Arc<dyn DnsClient>) -> Self {
        Self { dns_client }
    }

    /// Constructs the ACME challenge domain name.
    ///
    /// For a domain like `example.com`, returns `_acme-challenge.example.com`.
    pub fn challenge_domain(domain: &str) -> String {
        format!("_acme-challenge.{domain}")
    }

    /// Computes the DNS-01 value from a key authorization.
    ///
    /// This is the SHA-256 digest of the key authorization, base64url-encoded.
    pub fn dns_value(key_authorization: &KeyAuthorization) -> String {
        key_authorization.dns_value()
    }

    /// Sets the ACME TXT record for the given domain.
    pub async fn set_challenge_record(
        &self,
        domain: &str,
        key_authorization: &KeyAuthorization,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let challenge_domain = Self::challenge_domain(domain);
        let value = Self::dns_value(key_authorization);

        self.dns_client
            .update_record(&ferron_dns::DnsRecord {
                name: challenge_domain,
                record_type: ferron_dns::DnsRecordType::TXT,
                value,
                ttl: self.dns_client.minimum_ttl().max(60),
            })
            .await?;

        Ok(())
    }

    /// Removes the ACME TXT record for the given domain.
    pub async fn remove_challenge_record(
        &self,
        domain: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let challenge_domain = Self::challenge_domain(domain);
        self.dns_client
            .delete_record(&challenge_domain, "TXT")
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_challenge_domain() {
        assert_eq!(
            Dns01Helper::challenge_domain("example.com"),
            "_acme-challenge.example.com"
        );
    }

    #[test]
    fn test_challenge_domain_with_subdomain() {
        assert_eq!(
            Dns01Helper::challenge_domain("www.example.com"),
            "_acme-challenge.www.example.com"
        );
    }
}
