use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// Constellix DNS provider
  ConstellixDnsProvider,
  "Constellix",
  |challenge_params| dns_update::DnsUpdater::new_constellix(
    require_param(challenge_params, "api_key", "Constellix API key")?,
    require_param(challenge_params, "secret_key", "Constellix secret key")?,
    None,
  )
);
