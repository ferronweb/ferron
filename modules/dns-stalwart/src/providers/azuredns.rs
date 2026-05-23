use std::sync::Arc;

use dns_update::providers::azuredns::{AzureDnsConfig, AzureEnvironment};
use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::required_string;

pub struct AzureDnsProvider;

impl Provider<DnsContext<'static>> for AzureDnsProvider {
    fn name(&self) -> &'static str {
        "azuredns"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let tenant_id = required_string(ctx, "tenant_id", "azuredns", "tenant ID")?;
        let client_id = required_string(ctx, "client_id", "azuredns", "client ID")?;
        let client_secret = required_string(ctx, "client_secret", "azuredns", "client secret")?;
        let subscription_id =
            required_string(ctx, "subscription_id", "azuredns", "subscription ID")?;
        let resource_group = required_string(ctx, "resource_group", "azuredns", "resource group")?;

        let environment_name =
            required_string(ctx, "endpoint", "azuredns", "Azure environment name")?;
        let environment = match environment_name.as_str() {
            "AzurePublicCloud" => AzureEnvironment::Public,
            "AzureChinaCloud" => AzureEnvironment::China,
            "AzureUSGovernment" => AzureEnvironment::UsGovernment,
            _ => Err(anyhow::anyhow!(
                "Invalid Azure environment name for 'azuredns' DNS provider"
            ))?,
        };

        let config = AzureDnsConfig {
            tenant_id,
            client_id,
            client_secret,
            subscription_id,
            resource_group,
            environment,
            request_timeout: None,
        };

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_azuredns(config)?,
            1,
        )));
        Ok(())
    }
}
