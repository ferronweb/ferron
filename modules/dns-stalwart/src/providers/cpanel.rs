use std::collections::HashMap;
use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct CpanelDnsProvider;

impl Provider<DnsContext<'static>> for CpanelDnsProvider {
    fn name(&self) -> &'static str {
        "cpanel"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let base_url = ctx
            .config
            .get_value("base_url")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid base URL for 'cpanel' DNS provider"
            ))?;

        let username = ctx
            .config
            .get_value("username")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid username for 'cpanel' DNS provider"
            ))?;

        let token = ctx
            .config
            .get_value("token")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid token for 'cpanel' DNS provider"
            ))?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_cpanel(&base_url, &username, &token, None)?,
            300,
        )));
        Ok(())
    }
}
