use crate::dns_update_common::{dns_update_provider, optional_param, require_param};

dns_update_provider!(
  /// Tencent Cloud DNS DNS provider
  TencentCloudDnsProvider,
  "Tencent Cloud DNS",
  |challenge_params| dns_update::DnsUpdater::new_tencentcloud(
    require_param(challenge_params, "secret_id", "Tencent Cloud DNS secret ID")?,
    require_param(challenge_params, "secret_key", "Tencent Cloud DNS secret key")?,
    optional_param(challenge_params, "region"),
    optional_param(challenge_params, "session_token"),
    None,
  )
);
