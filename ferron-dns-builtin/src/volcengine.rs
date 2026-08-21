use crate::dns_update_common::{dns_update_provider, optional_param, require_param};

dns_update_provider!(
  /// Volcengine DNS provider
  VolcengineDnsProvider,
  "Volcengine",
  |challenge_params| dns_update::DnsUpdater::new_volcengine(dns_update::providers::volcengine::VolcengineConfig {
    access_key: require_param(challenge_params, "access_key", "Volcengine access key")?.to_string(),
    secret_key: require_param(challenge_params, "secret_key", "Volcengine secret key")?.to_string(),
    region: optional_param(challenge_params, "region").map(String::from),
    host: optional_param(challenge_params, "host").map(String::from),
    scheme: optional_param(challenge_params, "scheme").map(String::from),
    request_timeout: None,
  })
);
