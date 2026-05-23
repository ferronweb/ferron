use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::{opt_string, required_string};

pub struct UltraDnsDnsProvider;

impl Provider<DnsContext<'static>> for UltraDnsDnsProvider {
    fn name(&self) -> &'static str {
        "ultradns"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let username = required_string(ctx, "username", "ultradns", "username")?;
        let password = required_string(ctx, "password", "ultradns", "password")?;
        let endpoint = opt_string(ctx, "endpoint");

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_ultradns(&username, &password, endpoint, None)?,
            60,
        )));
        Ok(())
    }
}
