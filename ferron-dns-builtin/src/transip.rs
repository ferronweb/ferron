use crate::dns_update_common::{bool_param, dns_update_provider, require_param};

dns_update_provider!(
  /// TransIP DNS provider
  TransIpDnsProvider,
  "TransIP",
  |challenge_params| dns_update::DnsUpdater::new_transip(
    require_param(challenge_params, "login", "TransIP login")?,
    require_param(challenge_params, "private_key_pem", "TransIP private key PEM")?,
    bool_param(challenge_params, "global_key"),
    None,
  )
);
