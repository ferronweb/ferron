use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// IPv64.net DNS provider
  Ipv64DnsProvider,
  "IPv64.net",
  |challenge_params| dns_update::DnsUpdater::new_ipv64(
    require_param(challenge_params, "api_key", "IPv64.net API key")?,
    None,
  )
);
