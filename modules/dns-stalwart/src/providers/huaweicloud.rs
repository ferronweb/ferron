use std::collections::HashMap;
use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct HuaweiCloudDnsProvider;

impl Provider<DnsContext<'static>> for HuaweiCloudDnsProvider {
    fn name(&self) -> &'static str {
        "huaweicloud"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let access_key_id = ctx
            .config
            .get_value("access_key_id")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid access_key_id for 'huaweicloud' DNS provider"
            ))?;

        let access_key_secret = ctx
            .config
            .get_value("access_key_secret")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid access_key_secret for 'huaweicloud' DNS provider"
            ))?;

        let region = ctx
            .config
            .get_value("region")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid region for 'huaweicloud' DNS provider"
            ))?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_huaweicloud(&access_key_id, &access_key_secret, &region, None)?,
            1,
        )));
        Ok(())
    }
}
