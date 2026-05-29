use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::required_string;

pub struct ConstellixDnsProvider;

impl Provider<DnsContext<'static>> for ConstellixDnsProvider {
    fn name(&self) -> &'static str {
        "constellix"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let api_key = required_string(ctx, "api_key", "constellix")?;
        let secret_key = required_string(ctx, "secret_key", "constellix")?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_constellix(&api_key, &secret_key, None)?,
            30,
        )));
        Ok(())
    }
}
