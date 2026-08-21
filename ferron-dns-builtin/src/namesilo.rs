use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// NameSilo DNS provider
  NameSiloDnsProvider,
  "NameSilo",
  |challenge_params| dns_update::DnsUpdater::new_namesilo(
    require_param(challenge_params, "api_token", "NameSilo API key")?,
    None,
  )
);
