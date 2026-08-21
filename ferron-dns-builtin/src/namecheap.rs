use crate::dns_update_common::{dns_update_provider, optional_param, require_param};

dns_update_provider!(
  /// Namecheap DNS provider
  NamecheapDnsProvider,
  "Namecheap",
  |challenge_params| dns_update::DnsUpdater::new_namecheap(
    require_param(challenge_params, "api_key", "Namecheap API user")?,
    require_param(challenge_params, "api_secret", "Namecheap API key")?,
    require_param(challenge_params, "client_ip", "Namecheap client IP")?,
    optional_param(challenge_params, "username"),
    None,
  )
);
