use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// Huawei Cloud DNS DNS provider
  HuaweiCloudDnsProvider,
  "Huawei Cloud DNS",
  |challenge_params| dns_update::DnsUpdater::new_huaweicloud(
    require_param(challenge_params, "access_key_id", "Huawei Cloud DNS access key ID")?,
    require_param(challenge_params, "access_key_secret", "Huawei Cloud DNS access key secret")?,
    require_param(challenge_params, "region", "Huawei Cloud DNS region")?,
    None,
  )
);
