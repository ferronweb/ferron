use std::net::ToSocketAddrs;
use std::{collections::HashMap, error::Error};

use async_trait::async_trait;
use base64::Engine;
use dns_update::{DnsUpdater, TsigAlgorithm};

use ferron_common::dns::{separate_subdomain_from_domain_name, DnsProvider};

/// RFC2136 with TSIG DNS provider
pub struct Rfc2136DnsProvider {
  client: DnsUpdater,
}

impl Rfc2136DnsProvider {
  /// Create a new RFC2136 DNS provider
  fn new(
    addr: dns_update::providers::rfc2136::DnsAddress,
    key_name: &str,
    key: Vec<u8>,
    algorithm: TsigAlgorithm,
  ) -> dns_update::Result<Self> {
    Ok(Self {
      client: DnsUpdater::new_rfc2136_tsig(addr, key_name, key, algorithm)?,
    })
  }

  /// Load a RFC2136 DNS provider from ACME challenge parameters
  pub fn from_parameters(challenge_params: &HashMap<String, String>) -> Result<Self, Box<dyn Error + Send + Sync>> {
    let addr_str = challenge_params
      .get("server")
      .ok_or_else(|| anyhow::anyhow!("Missing RFC 2136 server address"))?;
    let addr_uri = addr_str
      .parse::<hyper::Uri>()
      .map_err(|e| anyhow::anyhow!("Invalid RFC 2136 server address: {}", e))?;
    let addr = match addr_uri.scheme_str() {
      Some("tcp") => dns_update::providers::rfc2136::DnsAddress::Tcp(
        addr_uri
          .authority()
          .ok_or_else(|| anyhow::anyhow!("Missing RFC 2136 server address hostname"))?
          .as_str()
          .to_socket_addrs()
          .map_err(|e| anyhow::anyhow!("Failed to resolve RFC 2136 server address: {}", e))?
          .next()
          .ok_or_else(|| anyhow::anyhow!("No RFC 2136 server addresses found"))?,
      ),
      Some("udp") => dns_update::providers::rfc2136::DnsAddress::Udp(
        addr_uri
          .authority()
          .ok_or_else(|| anyhow::anyhow!("Missing RFC 2136 server address hostname"))?
          .as_str()
          .to_socket_addrs()
          .map_err(|e| anyhow::anyhow!("Failed to resolve RFC 2136 server address: {}", e))?
          .next()
          .ok_or_else(|| anyhow::anyhow!("No RFC 2136 server addresses found"))?,
      ),
      _ => Err(anyhow::anyhow!("Invalid RFC 2136 server address scheme"))?,
    };
    let key_name = challenge_params
      .get("key_name")
      .ok_or_else(|| anyhow::anyhow!("Missing RFC 2136 key name"))?;
    let key = base64::engine::general_purpose::STANDARD
      .decode(
        challenge_params
          .get("key_secret")
          .ok_or_else(|| anyhow::anyhow!("Missing RFC 2136 key name"))?,
      )
      .map_err(|e| anyhow::anyhow!("Failed to decode RFC 2136 key: {}", e))?;
    let tsig_algorithm = match &challenge_params
      .get("key_algorithm")
      .ok_or_else(|| anyhow::anyhow!("Missing RFC 2136 TSIG algorithm"))?
      .to_uppercase() as &str
    {
      "HMAC-MD5" => dns_update::TsigAlgorithm::HmacMd5,
      "GSS" => dns_update::TsigAlgorithm::Gss,
      "HMAC-SHA1" => dns_update::TsigAlgorithm::HmacSha1,
      "HMAC-SHA224" => dns_update::TsigAlgorithm::HmacSha224,
      "HMAC-SHA256" => dns_update::TsigAlgorithm::HmacSha256,
      "HMAC-SHA256-128" => dns_update::TsigAlgorithm::HmacSha256_128,
      "HMAC-SHA384" => dns_update::TsigAlgorithm::HmacSha384,
      "HMAC-SHA384-192" => dns_update::TsigAlgorithm::HmacSha384_192,
      "HMAC-SHA512" => dns_update::TsigAlgorithm::HmacSha512,
      "HMAC-SHA512-256" => dns_update::TsigAlgorithm::HmacSha512_256,
      _ => Err(anyhow::anyhow!("Unsupported RFC 2136 TSIG algorithm"))?,
    };
    Ok(
      Self::new(addr, key_name, key, tsig_algorithm)
        .map_err(|e| anyhow::anyhow!("Failed to initalize RFC 2136 DNS provider: {}", e))?,
    )
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
      .delete(full_domain, domain_name, dns_update::DnsRecordType::TXT)
      .await
      .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(())
  }
}
