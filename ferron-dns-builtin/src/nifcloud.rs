use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// NIFCLOUD DNS provider
  NifcloudDnsProvider,
  "NIFCLOUD",
  |challenge_params| dns_update::DnsUpdater::new_nifcloud(
    require_param(challenge_params, "api_key", "NIFCLOUD API key")?,
    require_param(challenge_params, "api_secret", "NIFCLOUD API secret")?,
    None,
  )
);
