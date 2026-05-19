use std::collections::HashMap;
use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct NameDotComDnsProvider;

impl Provider<DnsContext<'static>> for NameDotComDnsProvider {
    fn name(&self) -> &'static str {
        "namedotcom"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let username = ctx
            .config
            .get_value("username")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid username for 'namedotcom' DNS provider"
            ))?;

        let api_token = ctx
            .config
            .get_value("api_token")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid API token for 'namedotcom' DNS provider"
            ))?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_namedotcom(&username, &api_token, None)?,
            600,
        )));
        Ok(())
    }
}
