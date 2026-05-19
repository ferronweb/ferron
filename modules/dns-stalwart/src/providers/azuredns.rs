use std::collections::HashMap;
use std::sync::Arc;

use dns_update::providers::azuredns::{AzureDnsConfig, AzureEnvironment};
use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct AzureDnsProvider;

impl Provider<DnsContext<'static>> for AzureDnsProvider {
    fn name(&self) -> &'static str {
        "azuredns"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let tenant_id = ctx
            .config
            .get_value("tenant_id")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid tenant ID for 'azuredns' DNS provider"
            ))?;

        let client_id = ctx
            .config
            .get_value("client_id")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid client ID for 'azuredns' DNS provider"
            ))?;

        let client_secret = ctx
            .config
            .get_value("client_secret")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid client secret for 'azuredns' DNS provider"
            ))?;

        let subscription_id = ctx
            .config
            .get_value("subscription_id")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid subscription ID for 'azuredns' DNS provider"
            ))?;

        let resource_group = ctx
            .config
            .get_value("resource_group")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid resource group for 'azuredns' DNS provider"
            ))?;

        let environment_name = ctx
            .config
            .get_value("endpoint")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid Azure environment name for 'azuredns' DNS provider"
            ))?;
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
            environment,
            resource_group,
            request_timeout: None,
        };

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_azuredns(config)?,
            1,
        )));
        Ok(())
    }
}
