use std::collections::HashMap;
use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct PleskDnsProvider;

impl Provider<DnsContext<'static>> for PleskDnsProvider {
    fn name(&self) -> &'static str {
        "plesk"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let base_url = ctx
            .config
            .get_value("base_url")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid base URL for 'plesk' DNS provider"
            ))?;

        let api_key = ctx
            .config
            .get_value("api_key")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid API key for 'plesk' DNS provider"
            ))?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_plesk(&base_url, &api_key, None)?,
            300,
        )));
        Ok(())
    }
}
