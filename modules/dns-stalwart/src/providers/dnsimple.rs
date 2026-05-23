use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::required_string;

pub struct DnsimpleDnsProvider;

impl Provider<DnsContext<'static>> for DnsimpleDnsProvider {
    fn name(&self) -> &'static str {
        "dnsimple"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let oauth_token = required_string(ctx, "oauth_token", "dnsimple", "OAuth token")?;
        let account_id = required_string(ctx, "account_id", "dnsimple", "account ID")?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_dnsimple(&oauth_token, &account_id, None)?,
            60,
        )));
        Ok(())
    }
}
