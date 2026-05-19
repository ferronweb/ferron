use std::collections::HashMap;
use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct VercelDnsProvider;

impl Provider<DnsContext<'static>> for VercelDnsProvider {
    fn name(&self) -> &'static str {
        "vercel"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let auth_token = ctx
            .config
            .get_value("auth_token")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid auth token for 'vercel' DNS provider"
            ))?;

        let team_id = ctx
            .config
            .get_value("team_id")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()));

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_vercel(&auth_token, team_id, None)?,
            60,
        )));
        Ok(())
    }
}
