use crate::dns_update_common::{dns_update_provider, optional_param, require_param};

dns_update_provider!(
  /// UltraDNS DNS provider
  UltraDnsDnsProvider,
  "UltraDNS",
  |challenge_params| dns_update::DnsUpdater::new_ultradns(
    require_param(challenge_params, "username", "UltraDNS username")?,
    require_param(challenge_params, "password", "UltraDNS password")?,
    optional_param(challenge_params, "endpoint").map(String::from),
    None,
  )
);
