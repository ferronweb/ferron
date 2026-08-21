use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// DDNSS.de DNS provider
  DdnssDnsProvider,
  "DDNSS.de",
  |challenge_params| dns_update::DnsUpdater::new_ddnss(
    require_param(challenge_params, "key", "DDNSS.de update key")?,
    None,
  )
);
