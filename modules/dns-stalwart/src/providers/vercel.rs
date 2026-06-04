use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::{opt_string, required_string};

pub struct VercelDnsProvider;

impl Provider<DnsContext<'static>> for VercelDnsProvider {
    fn name(&self) -> &'static str {
        "vercel"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let auth_token = required_string(ctx, "auth_token", "vercel")?;
        let team_id = opt_string(ctx, "team_id");

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_vercel(&auth_token, team_id, None)?,
            60,
        )));
        Ok(())
    }
}
