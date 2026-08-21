use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// freemyip.com DNS provider
  FreeMyIpDnsProvider,
  "freemyip.com",
  |challenge_params| dns_update::DnsUpdater::new_freemyip(
    require_param(challenge_params, "token", "freemyip.com token")?,
    None,
  )
);
