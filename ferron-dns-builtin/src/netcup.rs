use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// netcup DNS provider
  NetcupDnsProvider,
  "netcup",
  |challenge_params| dns_update::DnsUpdater::new_netcup(
    require_param(challenge_params, "customer_number", "netcup customer number")?,
    require_param(challenge_params, "api_key", "netcup API key")?,
    require_param(challenge_params, "api_password", "netcup API password")?,
    None,
  )
);
