use std::error::Error;

use async_trait::async_trait;
use hickory_resolver::{config::ResolverConfig, name_server::TokioConnectionProvider};

/// Trait for DNS providers used for DNS-01 ACME challenge.
#[async_trait]
pub trait DnsProvider {
  async fn set_acme_txt_record(
    &self,
    acme_challenge_identifier: &str,
    dns_value: &str,
  ) -> Result<(), Box<dyn Error + Send + Sync>>;

  #[allow(unused_variables)]
  async fn remove_acme_txt_record(&self, acme_challenge_identifier: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
    Ok(())
  }
}

/// Separates subdomain from domain name.
pub async fn separate_subdomain_from_domain_name(domain_name: &str) -> (String, String) {
  let parts: Vec<&str> = domain_name
    .strip_suffix(".")
    .unwrap_or(domain_name)
    .split('.')
    .collect();
  let resolver = hickory_resolver::Resolver::builder_tokio()
    .unwrap_or(hickory_resolver::Resolver::builder_with_config(
      ResolverConfig::default(),
      TokioConnectionProvider::default(),
    ))
    .build();

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
