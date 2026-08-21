use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// Name.com DNS provider
  NameDotComDnsProvider,
  "Name.com",
  |challenge_params| dns_update::DnsUpdater::new_namedotcom(
    require_param(challenge_params, "username", "Name.com username")?,
    require_param(challenge_params, "api_token", "Name.com API token")?,
    None,
  )
);
