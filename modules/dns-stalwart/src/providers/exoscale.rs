use std::collections::HashMap;
use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct ExoscaleDnsProvider;

impl Provider<DnsContext<'static>> for ExoscaleDnsProvider {
    fn name(&self) -> &'static str {
        "exoscale"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let api_key = ctx
            .config
            .get_value("api_key")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid API key for 'exoscale' DNS provider"
            ))?;

        let api_secret = ctx
            .config
            .get_value("api_secret")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid API secret for 'exoscale' DNS provider"
            ))?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_exoscale(&api_key, &api_secret, None)?,
            0,
        )));
        Ok(())
    }
}
