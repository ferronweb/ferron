use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// Websupport DNS provider
  WebsupportDnsProvider,
  "Websupport",
  |challenge_params| dns_update::DnsUpdater::new_websupport(
    require_param(challenge_params, "api_key", "Websupport API key")?,
    require_param(challenge_params, "secret", "Websupport API secret")?,
    None,
  )
);
