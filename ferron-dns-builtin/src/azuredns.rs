use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// Azure DNS DNS provider
  AzureDnsProvider,
  "Azure DNS",
  |challenge_params| {
    let environment = match require_param(challenge_params, "endpoint", "Azure DNS endpoint")? {
      "AzurePublicCloud" => dns_update::providers::azuredns::AzureEnvironment::Public,
      "AzureChinaCloud" => dns_update::providers::azuredns::AzureEnvironment::China,
      "AzureUSGovernment" => dns_update::providers::azuredns::AzureEnvironment::UsGovernment,
      invalid_environment => Err(anyhow::anyhow!(
        "Invalid Azure DNS endpoint: \"{invalid_environment}\""
      ))?,
    };
    dns_update::DnsUpdater::new_azuredns(dns_update::providers::azuredns::AzureDnsConfig {
      tenant_id: require_param(challenge_params, "tenant_id", "Azure DNS tenant ID")?.to_string(),
      client_id: require_param(challenge_params, "client_id", "Azure DNS client ID")?.to_string(),
      client_secret: require_param(challenge_params, "client_secret", "Azure DNS client secret")?.to_string(),
      subscription_id: require_param(challenge_params, "subscription_id", "Azure DNS subscription ID")?.to_string(),
      resource_group: require_param(challenge_params, "resource_group", "Azure DNS resource group")?.to_string(),
      environment,
      request_timeout: None,
    })
  }
);
