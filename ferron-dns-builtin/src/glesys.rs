use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// GleSYS DNS provider
  GlesysDnsProvider,
  "GleSYS",
  |challenge_params| dns_update::DnsUpdater::new_glesys(
    require_param(challenge_params, "api_user", "GleSYS API user")?,
    require_param(challenge_params, "api_key", "GleSYS API key")?,
    None,
  )
);
