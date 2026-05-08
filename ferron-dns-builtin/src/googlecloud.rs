use std::{collections::HashMap, error::Error};

use async_trait::async_trait;
use dns_update::DnsUpdater;

use ferron_common::dns::{separate_subdomain_from_domain_name, DnsProvider};

/// Google Cloud DNS provider
pub struct GoogleCloudDnsProvider {
  client: DnsUpdater,
}

impl GoogleCloudDnsProvider {
  /// Create a new Google Cloud DNS provider
  fn new(config: dns_update::providers::google_cloud_dns::GoogleCloudDnsConfig) -> dns_update::Result<Self> {
    Ok(Self {
      client: DnsUpdater::new_google_cloud_dns(config)?,
    })
  }

  /// Load a Google Cloud DNS provider from ACME challenge parameters
  pub fn from_parameters(challenge_params: &HashMap<String, String>) -> Result<Self, Box<dyn Error + Send + Sync>> {
    let service_account_json = challenge_params
      .get("service_account_json")
      .ok_or_else(|| anyhow::anyhow!("Missing Google Cloud service account JSON"))?
      .to_owned();
    let project_id = challenge_params
      .get("project_id")
      .ok_or_else(|| anyhow::anyhow!("Missing Google Cloud project ID"))?
      .to_owned();
    let managed_zone = challenge_params.get("managed_zone").map(ToOwned::to_owned);
    let private_zone = challenge_params
      .get("private_zone")
      .is_some_and(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "True" | "yes" | "Yes" | "YES"));
    let impersonate_service_account = challenge_params
      .get("impersonate_service_account")
      .map(ToOwned::to_owned);

    let config = dns_update::providers::google_cloud_dns::GoogleCloudDnsConfig {
      service_account_json,
      project_id,
      managed_zone,
      private_zone,
      impersonate_service_account,
      request_timeout: None,
    };

    Ok(Self::new(config).map_err(|e| anyhow::anyhow!("Failed to initalize Google Cloud DNS provider: {}", e))?)
  }
}

#[async_trait]
impl DnsProvider for GoogleCloudDnsProvider {
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
