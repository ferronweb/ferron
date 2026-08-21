use crate::dns_update_common::{dns_update_provider, optional_param, require_param};

dns_update_provider!(
  /// ClouDNS DNS provider
  ClouDnsDnsProvider,
  "ClouDNS",
  |challenge_params| dns_update::DnsUpdater::new_cloudns(
    optional_param(challenge_params, "auth_id"),
    optional_param(challenge_params, "sub_auth_id"),
    require_param(challenge_params, "password", "ClouDNS API password")?,
    None,
  )
);
