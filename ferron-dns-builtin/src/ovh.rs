use std::{collections::HashMap, error::Error};

use async_trait::async_trait;
use dns_update::{providers::ovh::OvhEndpoint, DnsUpdater};

use ferron_common::dns::{separate_subdomain_from_domain_name, DnsProvider};

/// OVH DNS provider
pub struct OvhDnsProvider {
  client: DnsUpdater,
}

impl OvhDnsProvider {
  /// Create a new OVH DNS provider
  fn new(
    application_key: &str,
    application_secret: &str,
    consumer_key: &str,
    endpoint: OvhEndpoint,
  ) -> dns_update::Result<Self> {
    Ok(Self {
      client: DnsUpdater::new_ovh(application_key, application_secret, consumer_key, endpoint, None)?,
    })
  }

  /// Load an OVH DNS provider from ACME challenge parameters
  pub fn from_parameters(challenge_params: &HashMap<String, String>) -> Result<Self, Box<dyn Error + Send + Sync>> {
    let application_key = challenge_params
      .get("application_key")
      .ok_or_else(|| anyhow::anyhow!("Missing OVH application key"))?;
    let application_secret = challenge_params
      .get("application_secret")
      .ok_or_else(|| anyhow::anyhow!("Missing OVH application secret"))?;
    let consumer_key = challenge_params
      .get("consumer_key")
      .ok_or_else(|| anyhow::anyhow!("Missing OVH consumer key"))?;
    let endpoint = challenge_params
      .get("endpoint")
      .ok_or_else(|| anyhow::anyhow!("Missing OVH endpoint name"))?;
    let endpoint = match endpoint.as_str() {
      "ovh-eu" => OvhEndpoint::OvhEu,
      "ovh-ca" => OvhEndpoint::OvhCa,
      "kimsufi-eu" => OvhEndpoint::KimsufiEu,
      "kimsufi-ca" => OvhEndpoint::KimsufiCa,
      "soyoustart-eu" => OvhEndpoint::SoyoustartCa,
      "soyoustart-ca" => OvhEndpoint::SoyoustartEu,
      _ => Err(anyhow::anyhow!("Invalid OVH endpoint name"))?,
    };
    Ok(
      Self::new(application_key, application_secret, consumer_key, endpoint)
        .map_err(|e| anyhow::anyhow!("Failed to initalize OVH DNS provider: {}", e))?,
    )
  }
}

#[async_trait]
impl DnsProvider for OvhDnsProvider {
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
      .delete(full_domain, domain_name, dns_update::DnsRecordType::TXT)
      .await
      .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(())
  }
}
