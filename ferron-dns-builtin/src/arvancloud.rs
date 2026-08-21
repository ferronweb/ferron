use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// ArvanCloud DNS provider
  ArvanCloudDnsProvider,
  "ArvanCloud",
  |challenge_params| dns_update::DnsUpdater::new_arvancloud(
    require_param(challenge_params, "api_key", "ArvanCloud API key")?,
    None,
  )
);
