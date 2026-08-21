use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// Gcore DNS provider
  GcoreDnsProvider,
  "Gcore",
  |challenge_params| dns_update::DnsUpdater::new_gcore(
    require_param(challenge_params, "api_token", "Gcore API token")?,
    None,
  )
);
