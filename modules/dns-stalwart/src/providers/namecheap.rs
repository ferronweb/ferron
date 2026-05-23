use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::{opt_string, required_string};

pub struct NamecheapDnsProvider;

impl Provider<DnsContext<'static>> for NamecheapDnsProvider {
    fn name(&self) -> &'static str {
        "namecheap"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let api_key = required_string(ctx, "api_key", "namecheap", "API key")?;
        let api_secret = required_string(ctx, "api_secret", "namecheap", "API secret")?;
        let client_ip = required_string(ctx, "client_ip", "namecheap", "client IP")?;
        let username = opt_string(ctx, "username");

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_namecheap(&api_key, &api_secret, &client_ip, username, None)?,
            60,
        )));
        Ok(())
    }
}
