use std::{collections::HashMap, error::Error};

use dns_update::{DnsRecord, DnsRecordType, DnsUpdater};

use ferron_common::dns::separate_subdomain_from_domain_name;

/// TTL used for ACME challenge TXT records created through the `dns-update` crate
pub(crate) const ACME_TXT_TTL: u32 = 300;

/// Determine the full domain name of the ACME challenge TXT record and the zone name
async fn acme_challenge_domain(acme_challenge_identifier: &str) -> (String, String) {
  let (subdomain, domain_name) = separate_subdomain_from_domain_name(acme_challenge_identifier).await;
  let subdomain = if subdomain.is_empty() {
    "_acme-challenge".to_string()
  } else {
    format!("_acme-challenge.{subdomain}")
  };
  (format!("{subdomain}.{domain_name}"), domain_name)
}

/// Add the ACME challenge TXT record value, keeping other values at the same name intact
pub(crate) async fn set_acme_txt_record(
  client: &DnsUpdater,
  acme_challenge_identifier: &str,
  dns_value: &str,
) -> Result<(), Box<dyn Error + Send + Sync>> {
  let (full_domain, domain_name) = acme_challenge_domain(acme_challenge_identifier).await;
  client
    .add_to_rrset(
      full_domain,
      DnsRecordType::TXT,
      ACME_TXT_TTL,
      vec![DnsRecord::TXT(dns_value.to_string())],
      domain_name,
    )
    .await
    .map_err(|e| anyhow::anyhow!("{e}"))?;
  Ok(())
}

/// Remove the ACME challenge TXT record set
pub(crate) async fn remove_acme_txt_record(
  client: &DnsUpdater,
  acme_challenge_identifier: &str,
) -> Result<(), Box<dyn Error + Send + Sync>> {
  let (full_domain, domain_name) = acme_challenge_domain(acme_challenge_identifier).await;
  client
    .set_rrset(full_domain, DnsRecordType::TXT, ACME_TXT_TTL, vec![], domain_name)
    .await
    .map_err(|e| anyhow::anyhow!("{e}"))?;
  Ok(())
}

/// Obtain a required ACME challenge parameter, or a "Missing ..." error if it's absent
pub(crate) fn require_param<'a>(
  challenge_params: &'a HashMap<String, String>,
  key: &str,
  description: &str,
) -> Result<&'a str, Box<dyn Error + Send + Sync>> {
  Ok(
    challenge_params
      .get(key)
      .map(|x| x as &str)
      .ok_or_else(|| anyhow::anyhow!("Missing {description}"))?,
  )
}

/// Obtain an optional ACME challenge parameter
#[allow(dead_code)] // Unused when the enabled providers don't have optional parameters
pub(crate) fn optional_param<'a>(challenge_params: &'a HashMap<String, String>, key: &str) -> Option<&'a str> {
  challenge_params.get(key).map(|x| x as &str)
}

/// Interpret an optional ACME challenge parameter as a boolean (absent means `false`)
#[allow(dead_code)] // Unused when the enabled providers don't have boolean parameters
pub(crate) fn bool_param(challenge_params: &HashMap<String, String>, key: &str) -> bool {
  challenge_params
    .get(key)
    .is_some_and(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "True" | "yes" | "Yes" | "YES"))
}

/// Define a DNS provider backed by the `dns-update` crate. The builder expression receives
/// the ACME challenge parameters and must evaluate to `dns_update::Result<DnsUpdater>`.
macro_rules! dns_update_provider {
  ($(#[$attr:meta])* $name:ident, $display:literal, |$params:ident| $builder:expr) => {
    $(#[$attr])*
    pub struct $name {
      client: dns_update::DnsUpdater,
    }

    impl $name {
      #[doc = concat!("Load a ", $display, " DNS provider from ACME challenge parameters")]
      pub fn from_parameters(
        $params: &std::collections::HashMap<String, String>,
      ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let client = ($builder)
          .map_err(|e| anyhow::anyhow!(concat!("Failed to initalize ", $display, " DNS provider: {}"), e))?;
        Ok(Self { client })
      }
    }

    #[async_trait::async_trait]
    impl ferron_common::dns::DnsProvider for $name {
      async fn set_acme_txt_record(
        &self,
        acme_challenge_identifier: &str,
        dns_value: &str,
      ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        crate::dns_update_common::set_acme_txt_record(&self.client, acme_challenge_identifier, dns_value).await
      }

      async fn remove_acme_txt_record(
        &self,
        acme_challenge_identifier: &str,
      ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        crate::dns_update_common::remove_acme_txt_record(&self.client, acme_challenge_identifier).await
      }
    }
  };
}

pub(crate) use dns_update_provider;

#[cfg(test)]
mod tests {
  use super::*;

  dns_update_provider!(
    /// Test DNS provider
    TestDnsProvider,
    "Test",
    |challenge_params| dns_update::DnsUpdater::new_hetzner(
      require_param(challenge_params, "api_token", "Test API token")?,
      None,
    )
  );

  #[test]
  fn test_from_parameters_missing_required() {
    let params = HashMap::new();
    let error = TestDnsProvider::from_parameters(&params)
      .err()
      .expect("expected an error");
    assert_eq!(error.to_string(), "Missing Test API token");
  }

  #[test]
  fn test_from_parameters_success() {
    let mut params = HashMap::new();
    params.insert("api_token".to_string(), "token".to_string());
    assert!(TestDnsProvider::from_parameters(&params).is_ok());
  }

  #[test]
  fn test_optional_and_bool_params() {
    let mut params = HashMap::new();
    params.insert("present".to_string(), "value".to_string());
    params.insert("enabled".to_string(), "true".to_string());
    params.insert("disabled".to_string(), "false".to_string());
    assert_eq!(optional_param(&params, "present"), Some("value"));
    assert_eq!(optional_param(&params, "absent"), None);
    assert!(bool_param(&params, "enabled"));
    assert!(!bool_param(&params, "disabled"));
    assert!(!bool_param(&params, "absent"));
  }
}
