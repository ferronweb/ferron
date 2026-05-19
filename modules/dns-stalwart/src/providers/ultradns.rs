use std::collections::HashMap;
use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct UltraDnsDnsProvider;

impl Provider<DnsContext<'static>> for UltraDnsDnsProvider {
    fn name(&self) -> &'static str {
        "ultradns"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let username = ctx
            .config
            .get_value("username")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid username for 'ultradns' DNS provider"
            ))?;

        let password = ctx
            .config
            .get_value("password")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid password for 'ultradns' DNS provider"
            ))?;

        let endpoint = ctx
            .config
            .get_value("endpoint")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()));

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_ultradns(&username, &password, endpoint, None)?,
            60,
        )));
        Ok(())
    }
}
