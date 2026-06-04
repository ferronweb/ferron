use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::required_string;

pub struct GoDaddyDnsProvider;

impl Provider<DnsContext<'static>> for GoDaddyDnsProvider {
    fn name(&self) -> &'static str {
        "godaddy"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let api_key = required_string(ctx, "api_key", "godaddy")?;
        let api_secret = required_string(ctx, "api_secret", "godaddy")?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_godaddy(&api_key, &api_secret, None)?,
            600,
        )));
        Ok(())
    }
}
