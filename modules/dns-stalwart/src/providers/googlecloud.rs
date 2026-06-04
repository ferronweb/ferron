use std::sync::Arc;

use dns_update::providers::google_cloud_dns::GoogleCloudDnsConfig;
use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::{opt_bool, opt_string, required_string};

pub struct GoogleCloudDnsProvider;

impl Provider<DnsContext<'static>> for GoogleCloudDnsProvider {
    fn name(&self) -> &'static str {
        "googlecloud"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let config = GoogleCloudDnsConfig {
            service_account_json: required_string(ctx, "service_account_json", "googlecloud")?,
            project_id: required_string(ctx, "project_id", "googlecloud")?,
            managed_zone: opt_string(ctx, "managed_zone"),
            private_zone: opt_bool(ctx, "private_zone").unwrap_or(false),
            impersonate_service_account: opt_string(ctx, "impersonate_service_account"),
            request_timeout: None,
        };

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_google_cloud_dns(config)?,
            60,
        )));
        Ok(())
    }
}
