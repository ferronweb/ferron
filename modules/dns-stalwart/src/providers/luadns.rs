use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::required_string;

pub struct LuaDnsDnsProvider;

impl Provider<DnsContext<'static>> for LuaDnsDnsProvider {
    fn name(&self) -> &'static str {
        "luadns"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let api_username = required_string(ctx, "api_username", "luadns", "API username")?;
        let api_token = required_string(ctx, "api_token", "luadns", "API token")?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_luadns(&api_username, &api_token, None)?,
            60,
        )));
        Ok(())
    }
}
