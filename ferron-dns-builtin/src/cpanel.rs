use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// cPanel DNS provider
  CpanelDnsProvider,
  "cPanel",
  |challenge_params| dns_update::DnsUpdater::new_cpanel(
    require_param(challenge_params, "base_url", "cPanel base URL")?,
    require_param(challenge_params, "username", "cPanel username")?,
    require_param(challenge_params, "token", "cPanel API token")?,
    None,
  )
);
