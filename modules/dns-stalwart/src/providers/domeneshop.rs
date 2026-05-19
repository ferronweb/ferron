use std::collections::HashMap;
use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct DomeneshopDnsProvider;

impl Provider<DnsContext<'static>> for DomeneshopDnsProvider {
    fn name(&self) -> &'static str {
        "domeneshop"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let api_token = ctx
            .config
            .get_value("api_token")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid API token for 'domeneshop' DNS provider"
            ))?;

        let api_secret = ctx
            .config
            .get_value("api_secret")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid API secret for 'domeneshop' DNS provider"
            ))?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_domeneshop(&api_token, &api_secret, None)?,
            300,
        )));
        Ok(())
    }
}
