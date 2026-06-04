use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::required_string;

pub struct ExoscaleDnsProvider;

impl Provider<DnsContext<'static>> for ExoscaleDnsProvider {
    fn name(&self) -> &'static str {
        "exoscale"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let api_key = required_string(ctx, "api_key", "exoscale")?;
        let api_secret = required_string(ctx, "api_secret", "exoscale")?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_exoscale(&api_key, &api_secret, None)?,
            0,
        )));
        Ok(())
    }
}
