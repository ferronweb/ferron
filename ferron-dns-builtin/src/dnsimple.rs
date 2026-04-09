use std::{collections::HashMap, error::Error};

use async_trait::async_trait;
use dns_update::DnsUpdater;

use ferron_common::dns::{separate_subdomain_from_domain_name, DnsProvider};

/// DNSimple DNS provider
pub struct DnsimpleDnsProvider {
  client: DnsUpdater,
}

impl DnsimpleDnsProvider {
  /// Create a new DNSimple DNS provider
  fn new(oauth_token: &str, account_id: &str) -> dns_update::Result<Self> {
    Ok(Self {
      client: DnsUpdater::new_dnsimple(oauth_token, account_id, None)?,
    })
  }

  /// Load a DNSimple DNS provider from ACME challenge parameters
  pub fn from_parameters(challenge_params: &HashMap<String, String>) -> Result<Self, Box<dyn Error + Send + Sync>> {
    let oauth_token = challenge_params
      .get("oauth_token")
      .ok_or_else(|| anyhow::anyhow!("Missing DNSimple OAuth token"))?;
    let account_id = challenge_params
      .get("account_id")
      .ok_or_else(|| anyhow::anyhow!("Missing DNSimple account ID"))?;
    Ok(
      Self::new(oauth_token, account_id)
        .map_err(|e| anyhow::anyhow!("Failed to initalize DNSimple DNS provider: {}", e))?,
    )
  }
}

#[async_trait]
impl DnsProvider for DnsimpleDnsProvider {
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
        dns_update::DnsRecord::TXT(dns_value.to_string()),
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
      .delete(full_domain, domain_name, dns_update::DnsRecordType::TXT)
      .await
      .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(())
  }
}
