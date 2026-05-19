use std::collections::HashMap;
use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct TencentCloudDnsProvider;

impl Provider<DnsContext<'static>> for TencentCloudDnsProvider {
    fn name(&self) -> &'static str {
        "tencentcloud"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let secret_id = ctx
            .config
            .get_value("secret_id")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid secret ID for 'tencentcloud' DNS provider"
            ))?;

        let secret_key = ctx
            .config
            .get_value("secret_key")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid secret key for 'tencentcloud' DNS provider"
            ))?;

        let region = ctx
            .config
            .get_value("region")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()));

        let session_token = ctx
            .config
            .get_value("session_token")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()));

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_tencentcloud(&secret_id, &secret_key, region, session_token, None)?,
            600,
        )));
        Ok(())
    }
}
