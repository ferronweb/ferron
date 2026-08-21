use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// hosting.de DNS provider
  HostingdeDnsProvider,
  "hosting.de",
  |challenge_params| dns_update::DnsUpdater::new_hostingde(
    require_param(challenge_params, "api_key", "hosting.de API key")?,
    None,
  )
);
