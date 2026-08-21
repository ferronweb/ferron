use crate::dns_update_common::{dns_update_provider, optional_param, require_param};

dns_update_provider!(
  /// Amazon Lightsail DNS provider
  LightsailDnsProvider,
  "Amazon Lightsail",
  |challenge_params| dns_update::DnsUpdater::new_lightsail(dns_update::providers::lightsail::LightsailConfig {
    access_key_id: require_param(challenge_params, "access_key_id", "Amazon Lightsail access key ID")?.to_string(),
    secret_access_key: require_param(challenge_params, "secret_access_key", "Amazon Lightsail secret access key")?
      .to_string(),
    session_token: optional_param(challenge_params, "session_token").map(String::from),
    region: optional_param(challenge_params, "region").map(String::from),
    domain: optional_param(challenge_params, "domain").map(String::from),
    request_timeout: None,
  })
);
