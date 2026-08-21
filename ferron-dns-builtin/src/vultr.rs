use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// Vultr DNS provider
  VultrDnsProvider,
  "Vultr",
  |challenge_params| dns_update::DnsUpdater::new_vultr(
    require_param(challenge_params, "api_key", "Vultr API key")?,
    None,
  )
);
