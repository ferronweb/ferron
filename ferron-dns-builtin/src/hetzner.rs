use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// Hetzner DNS provider
  HetznerDnsProvider,
  "Hetzner",
  |challenge_params| dns_update::DnsUpdater::new_hetzner(
    require_param(challenge_params, "api_token", "Hetzner API token")?,
    None,
  )
);
