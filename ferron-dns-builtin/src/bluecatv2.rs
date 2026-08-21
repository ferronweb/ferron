use crate::dns_update_common::{bool_param, dns_update_provider, require_param};

dns_update_provider!(
  /// BlueCat DNS provider
  BluecatV2DnsProvider,
  "BlueCat",
  |challenge_params| dns_update::DnsUpdater::new_bluecatv2(dns_update::providers::bluecatv2::BluecatV2Config {
    server_url: require_param(challenge_params, "server_url", "BlueCat server URL")?.to_string(),
    username: require_param(challenge_params, "username", "BlueCat username")?.to_string(),
    password: require_param(challenge_params, "password", "BlueCat password")?.to_string(),
    config_name: require_param(challenge_params, "config_name", "BlueCat configuration name")?.to_string(),
    view_name: require_param(challenge_params, "view_name", "BlueCat DNS view name")?.to_string(),
    skip_deploy: bool_param(challenge_params, "skip_deploy"),
    request_timeout: None,
  })
);
