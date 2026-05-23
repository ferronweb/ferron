use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::required_string;

pub struct GlesysDnsProvider;

impl Provider<DnsContext<'static>> for GlesysDnsProvider {
    fn name(&self) -> &'static str {
        "glesys"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let api_user = required_string(ctx, "api_user", "glesys", "API user")?;
        let api_key = required_string(ctx, "api_key", "glesys", "API key")?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_glesys(&api_user, &api_key, None)?,
            60,
        )));
        Ok(())
    }
}
