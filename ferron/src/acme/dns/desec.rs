use std::error::Error;

use async_trait::async_trait;

use crate::acme::dns::{separate_subdomain_from_domain_name, DnsProvider};

/// deSEC DNS provider
pub struct DesecDnsProvider {
  client: desec_api::Client,
}

impl DesecDnsProvider {
  /// Create a new deSEC DNS provider
  pub fn new(api_token: &str) -> Result<Self, desec_api::Error> {
    Ok(Self {
      client: desec_api::Client::new(api_token.to_string())?,
    })
  }
}

#[async_trait]
impl DnsProvider for DesecDnsProvider {
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
    self
      .client
      .rrset()
      .create_rrset(
        &domain_name,
        Some(&subdomain),
        "TXT",
        3600,
        &vec![format!("\"{dns_value}\"")],
      )
      .await?;
    Ok(())
  }

  async fn remove_acme_txt_record(&self, acme_challenge_identifier: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (subdomain, domain_name) = separate_subdomain_from_domain_name(acme_challenge_identifier).await;
    let subdomain = if subdomain.is_empty() {
      "_acme-challenge".to_string()
    } else {
      format!("_acme-challenge.{subdomain}")
    };
    self
      .client
      .rrset()
      .delete_rrset(&domain_name, Some(&subdomain), "TXT")
      .await?;
    Ok(())
  }
}
