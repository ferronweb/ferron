use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// IONOS DNS provider
  IonosDnsProvider,
  "IONOS",
  |challenge_params| dns_update::DnsUpdater::new_ionos(
    require_param(challenge_params, "api_key", "IONOS API key")?,
    None,
  )
);
