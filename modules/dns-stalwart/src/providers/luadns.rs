use std::collections::HashMap;
use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct LuaDnsDnsProvider;

impl Provider<DnsContext<'static>> for LuaDnsDnsProvider {
    fn name(&self) -> &'static str {
        "luadns"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let api_username = ctx
            .config
            .get_value("api_username")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid API username for 'luadns' DNS provider"
            ))?;

        let api_token = ctx
            .config
            .get_value("api_token")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid API token for 'luadns' DNS provider"
            ))?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_luadns(&api_username, &api_token, None)?,
            60,
        )));
        Ok(())
    }
}
