use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// LuaDNS DNS provider
  LuaDnsDnsProvider,
  "LuaDNS",
  |challenge_params| dns_update::DnsUpdater::new_luadns(
    require_param(challenge_params, "api_username", "LuaDNS API username")?,
    require_param(challenge_params, "api_token", "LuaDNS API token")?,
    None,
  )
);
