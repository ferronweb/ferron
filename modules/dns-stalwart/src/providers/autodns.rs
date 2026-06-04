use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::required_string;

pub struct AutoDNSProvider;

impl Provider<DnsContext<'static>> for AutoDNSProvider {
    fn name(&self) -> &'static str {
        "autodns"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let username = required_string(ctx, "username", "autodns")?;
        let password = required_string(ctx, "password", "autodns")?;
        let context = ctx
            .config
            .get_value("context")
            .and_then(|v| v.as_number())
            .map(|n| n as u32);

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_autodns(&username, &password, context, None)?,
            300,
        )));
        Ok(())
    }
}
