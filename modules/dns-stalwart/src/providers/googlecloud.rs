use std::{collections::HashMap, sync::Arc};

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct GoogleCloudDnsProvider;

impl Provider<DnsContext<'static>> for GoogleCloudDnsProvider {
    fn name(&self) -> &'static str {
        "googlecloud"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let service_account_json = ctx
            .config
            .get_value("service_account_json")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid service account JSON for 'googlecloud' DNS provider"
            ))?;
        let project_id = ctx
            .config
            .get_value("project_id")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid project ID for 'googlecloud' DNS provider"
            ))?;
        let managed_zone = ctx
            .config
            .get_value("managed_zone")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()));
        let private_zone = ctx
            .config
            .get_value("private_zone")
            .and_then(|v| v.as_boolean())
            .unwrap_or(false);
        let impersonate_service_account = ctx
            .config
            .get_value("impersonate_service_account")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()));

        let config = dns_update::providers::google_cloud_dns::GoogleCloudDnsConfig {
            service_account_json,
            project_id,
            managed_zone,
            private_zone,
            impersonate_service_account,
            request_timeout: None,
        };

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_google_cloud_dns(config)?,
            60,
        )));
        Ok(())
    }
}
