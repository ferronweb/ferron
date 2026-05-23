use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::required_string;

pub struct PleskDnsProvider;

impl Provider<DnsContext<'static>> for PleskDnsProvider {
    fn name(&self) -> &'static str {
        "plesk"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let base_url = required_string(ctx, "base_url", "plesk", "base URL")?;
        let api_key = required_string(ctx, "api_key", "plesk", "API key")?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_plesk(&base_url, &api_key, None)?,
            300,
        )));
        Ok(())
    }
}
