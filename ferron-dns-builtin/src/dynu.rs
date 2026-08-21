use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// Dynu DNS provider
  DynuDnsProvider,
  "Dynu",
  |challenge_params| dns_update::DnsUpdater::new_dynu(
    require_param(challenge_params, "api_key", "Dynu API key")?,
    None,
  )
);
