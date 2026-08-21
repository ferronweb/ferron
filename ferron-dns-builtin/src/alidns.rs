use crate::dns_update_common::{dns_update_provider, optional_param, require_param};

dns_update_provider!(
  /// Alibaba Cloud DNS DNS provider
  AlidnsDnsProvider,
  "Alibaba Cloud DNS",
  |challenge_params| dns_update::DnsUpdater::new_alidns(
    require_param(challenge_params, "access_key_id", "Alibaba Cloud DNS access key ID")?,
    require_param(challenge_params, "access_key_secret", "Alibaba Cloud DNS access key secret")?,
    optional_param(challenge_params, "region"),
    optional_param(challenge_params, "security_token"),
    optional_param(challenge_params, "line"),
    None,
  )
);
