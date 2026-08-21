use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// GoDaddy DNS provider
  GoDaddyDnsProvider,
  "GoDaddy",
  |challenge_params| dns_update::DnsUpdater::new_godaddy(
    require_param(challenge_params, "api_key", "GoDaddy API key")?,
    require_param(challenge_params, "api_secret", "GoDaddy API secret")?,
    None,
  )
);
