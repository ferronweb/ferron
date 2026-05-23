use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::required_string;

pub struct NetcupDnsProvider;

impl Provider<DnsContext<'static>> for NetcupDnsProvider {
    fn name(&self) -> &'static str {
        "netcup"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let customer_number =
            required_string(ctx, "customer_number", "netcup", "customer number")?;
        let api_key = required_string(ctx, "api_key", "netcup", "API key")?;
        let api_password = required_string(ctx, "api_password", "netcup", "API password")?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_netcup(&customer_number, &api_key, &api_password, None)?,
            60,
        )));
        Ok(())
    }
}
