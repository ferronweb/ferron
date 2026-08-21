use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// Infomaniak DNS provider
  InfomaniakDnsProvider,
  "Infomaniak",
  |challenge_params| dns_update::DnsUpdater::new_infomaniak(
    require_param(challenge_params, "api_token", "Infomaniak API token")?,
    None,
  )
);
