use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::required_string;

pub struct NameDotComDnsProvider;

impl Provider<DnsContext<'static>> for NameDotComDnsProvider {
    fn name(&self) -> &'static str {
        "namedotcom"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let username = required_string(ctx, "username", "namedotcom", "username")?;
        let api_token = required_string(ctx, "api_token", "namedotcom", "API token")?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_namedotcom(&username, &api_token, None)?,
            600,
        )));
        Ok(())
    }
}
