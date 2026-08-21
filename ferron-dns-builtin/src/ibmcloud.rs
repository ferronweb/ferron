use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// IBM Cloud DNS DNS provider
  IbmCloudDnsProvider,
  "IBM Cloud DNS",
  |challenge_params| dns_update::DnsUpdater::new_ibmcloud(
    require_param(challenge_params, "username", "IBM Cloud DNS username")?,
    require_param(challenge_params, "api_key", "IBM Cloud DNS API key")?,
    None,
  )
);
