use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// easyDNS DNS provider
  EasyDnsDnsProvider,
  "easyDNS",
  |challenge_params| dns_update::DnsUpdater::new_easydns(
    require_param(challenge_params, "token", "easyDNS API token")?,
    require_param(challenge_params, "key", "easyDNS API key")?,
    None,
  )
);
