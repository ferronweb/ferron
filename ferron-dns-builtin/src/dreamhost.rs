use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// DreamHost DNS provider
  DreamHostDnsProvider,
  "DreamHost",
  |challenge_params| dns_update::DnsUpdater::new_dreamhost(
    require_param(challenge_params, "api_key", "DreamHost API key")?,
    None,
  )
);
