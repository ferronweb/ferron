use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::required_string;

pub struct CpanelDnsProvider;

impl Provider<DnsContext<'static>> for CpanelDnsProvider {
    fn name(&self) -> &'static str {
        "cpanel"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let base_url = required_string(ctx, "base_url", "cpanel", "base URL")?;
        let username = required_string(ctx, "username", "cpanel", "username")?;
        let token = required_string(ctx, "token", "cpanel", "token")?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_cpanel(&base_url, &username, &token, None)?,
            300,
        )));
        Ok(())
    }
}
