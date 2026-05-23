use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::required_string;

pub struct MythicBeastsDnsProvider;

impl Provider<DnsContext<'static>> for MythicBeastsDnsProvider {
    fn name(&self) -> &'static str {
        "mythicbeasts"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let username = required_string(ctx, "username", "mythicbeasts", "username")?;
        let password = required_string(ctx, "password", "mythicbeasts", "password")?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_mythicbeasts(&username, &password, None)?,
            60,
        )));
        Ok(())
    }
}
