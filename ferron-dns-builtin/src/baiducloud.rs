use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// Baidu Cloud DNS DNS provider
  BaiduCloudDnsProvider,
  "Baidu Cloud DNS",
  |challenge_params| dns_update::DnsUpdater::new_baiducloud(
    require_param(challenge_params, "access_key_id", "Baidu Cloud DNS access key ID")?,
    require_param(challenge_params, "access_key_secret", "Baidu Cloud DNS access key secret")?,
    None,
  )
);
