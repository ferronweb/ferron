use std::collections::HashMap;
use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct NetlifyDnsProvider;

impl Provider<DnsContext<'static>> for NetlifyDnsProvider {
    fn name(&self) -> &'static str {
        "netlify"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let access_token = ctx
            .config
            .get_value("access_token")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid access token for 'netlify' DNS provider"
            ))?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_netlify(&access_token, None)?,
            60,
        )));
        Ok(())
    }
}
