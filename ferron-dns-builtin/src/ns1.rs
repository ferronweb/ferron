use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// NS1 DNS provider
  Ns1DnsProvider,
  "NS1",
  |challenge_params| dns_update::DnsUpdater::new_ns1(
    require_param(challenge_params, "api_key", "NS1 API key")?,
    None,
  )
);
