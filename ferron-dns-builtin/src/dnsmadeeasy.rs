use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// DNS Made Easy DNS provider
  DnsMadeEasyDnsProvider,
  "DNS Made Easy",
  |challenge_params| dns_update::DnsUpdater::new_dnsmadeeasy(
    require_param(challenge_params, "api_key", "DNS Made Easy API key")?,
    require_param(challenge_params, "api_secret", "DNS Made Easy API secret")?,
    None,
  )
);
