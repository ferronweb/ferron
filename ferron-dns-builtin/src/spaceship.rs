use std::{collections::HashMap, error::Error};

use async_trait::async_trait;
use dns_update::DnsUpdater;

use ferron_common::dns::{separate_subdomain_from_domain_name, DnsProvider};

/// Spaceship DNS provider
pub struct SpaceshipDnsProvider {
  client: DnsUpdater,
}

impl SpaceshipDnsProvider {
  /// Create a new Spaceship DNS provider
  fn new(api_key: &str, secret_key: &str) -> dns_update::Result<Self> {
    Ok(Self {
      client: DnsUpdater::new_spaceship(api_key, secret_key, None)?,
    })
  }

  /// Load a Spaceship DNS provider from ACME challenge parameters
  pub fn from_parameters(challenge_params: &HashMap<String, String>) -> Result<Self, Box<dyn Error + Send + Sync>> {
    let api_key = challenge_params
      .get("api_key")
      .ok_or_else(|| anyhow::anyhow!("Missing Spaceship API key"))?;
    let api_secret = challenge_params
      .get("api_secret")
      .ok_or_else(|| anyhow::anyhow!("Missing Spaceship secret key"))?;
    Ok(
      Self::new(api_key, api_secret)
        .map_err(|e| anyhow::anyhow!("Failed to initalize Spaceship DNS provider: {}", e))?,
    )
  }
}

#[async_trait]
impl DnsProvider for SpaceshipDnsProvider {
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
        1200,
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
