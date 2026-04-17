use std::{collections::HashMap, sync::Arc};

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct DigitalOceanDnsProvider;

impl Provider<DnsContext<'static>> for DigitalOceanDnsProvider {
    fn name(&self) -> &'static str {
        "digitalocean"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let oauth_token = ctx
            .config
            .get_value("oauth_token")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid OAuth token for 'digitalocean' DNS provider"
            ))?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_digitalocean(&oauth_token, None)?,
            30,
        )));
        Ok(())
    }
}
