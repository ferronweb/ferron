use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::required_string;

pub struct WebSupportDnsProvider;

impl Provider<DnsContext<'static>> for WebSupportDnsProvider {
    fn name(&self) -> &'static str {
        "websupport"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let api_key = required_string(ctx, "api_key", "websupport")?;
        let secret = required_string(ctx, "secret", "websupport")?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_websupport(&api_key, &secret, None)?,
            300,
        )));
        Ok(())
    }
}
