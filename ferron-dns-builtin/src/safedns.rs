use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// SafeDNS DNS provider
  SafeDnsDnsProvider,
  "SafeDNS",
  |challenge_params| dns_update::DnsUpdater::new_safedns(
    require_param(challenge_params, "auth_token", "SafeDNS authentication token")?,
    None,
  )
);
