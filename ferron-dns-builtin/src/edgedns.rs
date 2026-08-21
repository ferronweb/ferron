use crate::dns_update_common::{dns_update_provider, optional_param, require_param};

dns_update_provider!(
  /// Akamai Edge DNS DNS provider
  EdgeDnsProvider,
  "Akamai Edge DNS",
  |challenge_params| dns_update::DnsUpdater::new_edgedns(dns_update::providers::edgedns::EdgeDnsConfig {
    host: require_param(challenge_params, "host", "Akamai Edge DNS host")?.to_string(),
    client_token: require_param(challenge_params, "client_token", "Akamai Edge DNS client token")?.to_string(),
    client_secret: require_param(challenge_params, "client_secret", "Akamai Edge DNS client secret")?.to_string(),
    access_token: require_param(challenge_params, "access_token", "Akamai Edge DNS access token")?.to_string(),
    account_switch_key: optional_param(challenge_params, "account_switch_key").map(String::from),
    request_timeout: None,
  })
);
