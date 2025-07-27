use std::{collections::HashMap, error::Error};

use async_trait::async_trait;
use porkbun_api::{CreateOrEditDnsRecord, DnsRecordType};

use ferron_common::dns::{separate_subdomain_from_domain_name, DnsProvider};

/// Porkbun DNS provider
pub struct PorkbunDnsProvider {
  client: porkbun_api::Client<porkbun_api::transport::DefaultTransport>,
}

impl PorkbunDnsProvider {
  /// Create a new Porkbun DNS provider
  fn new(api_key: &str, secret_key: &str) -> Self {
    let api_key = porkbun_api::ApiKey::new(secret_key, api_key);
    let client = porkbun_api::Client::new(api_key);
    Self { client }
  }

  /// Load a Porkbun DNS provider from ACME challenge parameters
  pub fn from_parameters(challenge_params: &HashMap<String, String>) -> Result<Self, Box<dyn Error + Send + Sync>> {
    let api_key = challenge_params
      .get("api_key")
      .ok_or_else(|| anyhow::anyhow!("Missing Porkbun API key"))?;
    let secret_key = challenge_params
      .get("secret_key")
      .ok_or_else(|| anyhow::anyhow!("Missing Porkbun secret key"))?;
    Ok(Self::new(api_key, secret_key))
  }
}

#[async_trait]
impl DnsProvider for PorkbunDnsProvider {
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
    let record = CreateOrEditDnsRecord::new(Some(&subdomain), DnsRecordType::TXT, dns_value);
    self.client.create(&domain_name, record).await?;
    Ok(())
  }

  async fn remove_acme_txt_record(&self, acme_challenge_identifier: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (subdomain, domain_name) = separate_subdomain_from_domain_name(acme_challenge_identifier).await;
    let subdomain = if subdomain.is_empty() {
      "_acme-challenge".to_string()
    } else {
      format!("_acme-challenge.{subdomain}")
    };
    for dns_entry in self.client.get_all(&domain_name).await? {
      if dns_entry.name == format!("{subdomain}.{domain_name}") && dns_entry.record_type == DnsRecordType::TXT {
        self.client.delete(&domain_name, &dns_entry.id).await?;
      }
    }
    Ok(())
  }
}
