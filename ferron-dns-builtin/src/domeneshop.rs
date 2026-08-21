use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// Domeneshop DNS provider
  DomeneshopDnsProvider,
  "Domeneshop",
  |challenge_params| dns_update::DnsUpdater::new_domeneshop(
    require_param(challenge_params, "api_token", "Domeneshop API token")?,
    require_param(challenge_params, "api_secret", "Domeneshop API secret")?,
    None,
  )
);
