use std::collections::HashMap;
use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct NamecheapDnsProvider;

impl Provider<DnsContext<'static>> for NamecheapDnsProvider {
    fn name(&self) -> &'static str {
        "namecheap"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let api_key = ctx
            .config
            .get_value("api_key")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid API key for 'namecheap' DNS provider"
            ))?;

        let api_secret = ctx
            .config
            .get_value("api_secret")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid API secret for 'namecheap' DNS provider"
            ))?;

        let client_ip = ctx
            .config
            .get_value("client_ip")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid client IP for 'namecheap' DNS provider"
            ))?;

        let username = ctx
            .config
            .get_value("username")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()));

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_namecheap(&api_key, &api_secret, &client_ip, username, None)?,
            60,
        )));
        Ok(())
    }
}
