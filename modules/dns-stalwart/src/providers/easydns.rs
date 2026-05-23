use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::required_string;

pub struct EasyDNSProvider;

impl Provider<DnsContext<'static>> for EasyDNSProvider {
    fn name(&self) -> &'static str {
        "easydns"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let token = required_string(ctx, "token", "easydns", "token")?;
        let key = required_string(ctx, "key", "easydns", "key")?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_easydns(&token, &key, None)?,
            300,
        )));
        Ok(())
    }
}
