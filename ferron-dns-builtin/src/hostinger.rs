use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// Hostinger DNS provider
  HostingerDnsProvider,
  "Hostinger",
  |challenge_params| dns_update::DnsUpdater::new_hostinger(
    require_param(challenge_params, "api_token", "Hostinger API token")?,
    None,
  )
);
