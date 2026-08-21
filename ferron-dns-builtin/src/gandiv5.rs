use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// Gandi DNS provider
  GandiV5DnsProvider,
  "Gandi",
  |challenge_params| dns_update::DnsUpdater::new_gandiv5(
    require_param(challenge_params, "personal_access_token", "Gandi personal access token")?,
    None,
  )
);
