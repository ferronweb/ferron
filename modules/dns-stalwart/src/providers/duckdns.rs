use std::collections::HashMap;
use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct DuckDNSProvider;

impl Provider<DnsContext<'static>> for DuckDNSProvider {
    fn name(&self) -> &'static str {
        "duckdns"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let token = ctx
            .config
            .get_value("token")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid token for 'duckdns' DNS provider"
            ))?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_duckdns(&token, None)?,
            60,
        )));
        Ok(())
    }
}
