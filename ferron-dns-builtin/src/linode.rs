use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// Linode DNS provider
  LinodeDnsProvider,
  "Linode",
  |challenge_params| dns_update::DnsUpdater::new_linode(
    require_param(challenge_params, "api_token", "Linode API token")?,
    None,
  )
);
