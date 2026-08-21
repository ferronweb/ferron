use crate::dns_update_common::{bool_param, dns_update_provider, require_param};

dns_update_provider!(
  /// INWX DNS provider
  InwxDnsProvider,
  "INWX",
  |challenge_params| dns_update::DnsUpdater::new_inwx(
    require_param(challenge_params, "username", "INWX username")?,
    require_param(challenge_params, "password", "INWX password")?,
    bool_param(challenge_params, "sandbox"),
    None,
  )
);
