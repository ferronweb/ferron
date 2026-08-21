use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// Simply.com DNS provider
  SimplyComDnsProvider,
  "Simply.com",
  |challenge_params| dns_update::DnsUpdater::new_simplycom(
    require_param(challenge_params, "account_name", "Simply.com account name")?,
    require_param(challenge_params, "api_key", "Simply.com API key")?,
    None,
  )
);
