use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// Netlify DNS provider
  NetlifyDnsProvider,
  "Netlify",
  |challenge_params| dns_update::DnsUpdater::new_netlify(
    require_param(challenge_params, "access_token", "Netlify access token")?,
    None,
  )
);
