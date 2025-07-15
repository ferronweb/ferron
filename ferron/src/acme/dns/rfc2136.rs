use std::error::Error;

use async_trait::async_trait;
use dns_update::{DnsUpdater, TsigAlgorithm};

use crate::acme::dns::{separate_subdomain_from_domain_name, DnsProvider};

/// RFC2136 with TSIG DNS provider
pub struct Rfc2136DnsProvider {
  client: DnsUpdater,
}

impl Rfc2136DnsProvider {
  /// Create a new RFC2136 DNS provider
  pub fn new(
    addr: dns_update::providers::rfc2136::DnsAddress,
    key_name: &str,
    key: Vec<u8>,
    algorithm: TsigAlgorithm,
  ) -> dns_update::Result<Self> {
    Ok(Self {
      client: DnsUpdater::new_rfc2136_tsig(addr, key_name, key, algorithm)?,
    })
  }
}

#[async_trait]
impl DnsProvider for Rfc2136DnsProvider {
  async fn set_acme_txt_record(
    &self,
    acme_challenge_identifier: &str,
    dns_value: &str,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (subdomain, domain_name) = separate_subdomain_from_domain_name(acme_challenge_identifier).await;
    let subdomain = if subdomain.is_empty() {
      "_acme-challenge".to_string()
    } else {
      format!("_acme-challenge.{subdomain}")
    };
    let full_domain = format!("{subdomain}.{domain_name}");
    self
      .client
      .create(
        full_domain,
        dns_update::DnsRecord::TXT {
          content: dns_value.to_string(),
        },
        300,
        domain_name,
      )
      .await
      .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(())
  }

  async fn remove_acme_txt_record(&self, acme_challenge_identifier: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (subdomain, domain_name) = separate_subdomain_from_domain_name(acme_challenge_identifier).await;
    let subdomain = if subdomain.is_empty() {
      "_acme-challenge".to_string()
    } else {
      format!("_acme-challenge.{subdomain}")
    };
    let full_domain = format!("{subdomain}.{domain_name}");
    self
      .client
      .delete(full_domain, domain_name)
      .await
      .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(())
  }
}
