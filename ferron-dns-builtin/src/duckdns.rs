use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// Duck DNS DNS provider
  DuckDnsDnsProvider,
  "Duck DNS",
  |challenge_params| dns_update::DnsUpdater::new_duckdns(
    require_param(challenge_params, "token", "Duck DNS token")?,
    None,
  )
);
