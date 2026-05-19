use std::collections::HashMap;
use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct NifcloudDnsProvider;

impl Provider<DnsContext<'static>> for NifcloudDnsProvider {
    fn name(&self) -> &'static str {
        "nifcloud"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let api_key = ctx
            .config
            .get_value("api_key")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid API key for 'nifcloud' DNS provider"
            ))?;

        let api_secret = ctx
            .config
            .get_value("api_secret")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid API secret for 'nifcloud' DNS provider"
            ))?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_nifcloud(&api_key, &api_secret, None)?,
            60,
        )));
        Ok(())
    }
}
