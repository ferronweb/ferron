use crate::dns_update_common::{dns_update_provider, optional_param, require_param};

dns_update_provider!(
  /// Infoblox DNS provider
  InfobloxDnsProvider,
  "Infoblox",
  |challenge_params| dns_update::DnsUpdater::new_infoblox(dns_update::providers::infoblox::InfobloxConfig {
    host: require_param(challenge_params, "host", "Infoblox host")?.to_string(),
    port: optional_param(challenge_params, "port").map(String::from),
    username: require_param(challenge_params, "username", "Infoblox username")?.to_string(),
    password: require_param(challenge_params, "password", "Infoblox password")?.to_string(),
    wapi_version: optional_param(challenge_params, "wapi_version").map(String::from),
    dns_view: optional_param(challenge_params, "dns_view").map(String::from),
    request_timeout: None,
  })
);
