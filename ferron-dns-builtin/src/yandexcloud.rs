use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// Yandex Cloud DNS DNS provider
  YandexCloudDnsProvider,
  "Yandex Cloud DNS",
  |challenge_params| dns_update::DnsUpdater::new_yandexcloud(dns_update::providers::yandexcloud::YandexCloudConfig {
    iam_token_b64: require_param(challenge_params, "iam_token_b64", "Yandex Cloud DNS base64-encoded IAM token")?
      .to_string(),
    folder_id: require_param(challenge_params, "folder_id", "Yandex Cloud DNS folder ID")?.to_string(),
    request_timeout: None,
  })
);
