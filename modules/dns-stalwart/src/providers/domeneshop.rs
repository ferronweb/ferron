use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::required_string;

pub struct DomeneshopDnsProvider;

impl Provider<DnsContext<'static>> for DomeneshopDnsProvider {
    fn name(&self) -> &'static str {
        "domeneshop"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let api_token = required_string(ctx, "api_token", "domeneshop")?;
        let api_secret = required_string(ctx, "api_secret", "domeneshop")?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_domeneshop(&api_token, &api_secret, None)?,
            300,
        )));
        Ok(())
    }
}
