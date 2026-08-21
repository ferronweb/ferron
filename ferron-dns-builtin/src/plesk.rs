use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// Plesk DNS provider
  PleskDnsProvider,
  "Plesk",
  |challenge_params| dns_update::DnsUpdater::new_plesk(
    require_param(challenge_params, "base_url", "Plesk base URL")?,
    require_param(challenge_params, "api_key", "Plesk API key")?,
    None,
  )
);
