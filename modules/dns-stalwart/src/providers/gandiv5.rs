use std::collections::HashMap;
use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct GandiV5DnsProvider;

impl Provider<DnsContext<'static>> for GandiV5DnsProvider {
    fn name(&self) -> &'static str {
        "gandiv5"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let personal_access_token = ctx
            .config
            .get_value("personal_access_token")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid personal access token for 'gandiv5' DNS provider"
            ))?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_gandiv5(&personal_access_token, None)?,
            300,
        )));
        Ok(())
    }
}
