use crate::dns_update_common::{dns_update_provider, optional_param, require_param};

dns_update_provider!(
  /// Vercel DNS provider
  VercelDnsProvider,
  "Vercel",
  |challenge_params| dns_update::DnsUpdater::new_vercel(
    require_param(challenge_params, "auth_token", "Vercel authentication token")?,
    optional_param(challenge_params, "team_id"),
    None,
  )
);
