use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// Scaleway DNS provider
  ScalewayDnsProvider,
  "Scaleway",
  |challenge_params| dns_update::DnsUpdater::new_scaleway(
    require_param(challenge_params, "api_token", "Scaleway API token")?,
    None,
  )
);
