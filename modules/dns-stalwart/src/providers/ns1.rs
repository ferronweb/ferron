use std::collections::HashMap;
use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct NS1Provider;

impl Provider<DnsContext<'static>> for NS1Provider {
    fn name(&self) -> &'static str {
        "ns1"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let api_key = ctx
            .config
            .get_value("api_key")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid API key for 'ns1' DNS provider"
            ))?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_ns1(&api_key, None)?,
            0,
        )));
        Ok(())
    }
}
