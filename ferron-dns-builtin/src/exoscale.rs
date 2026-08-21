use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// Exoscale DNS provider
  ExoscaleDnsProvider,
  "Exoscale",
  |challenge_params| dns_update::DnsUpdater::new_exoscale(
    require_param(challenge_params, "api_key", "Exoscale API key")?,
    require_param(challenge_params, "api_secret", "Exoscale API secret")?,
    None,
  )
);
