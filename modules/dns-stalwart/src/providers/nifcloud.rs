use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::required_string;

pub struct NifcloudDnsProvider;

impl Provider<DnsContext<'static>> for NifcloudDnsProvider {
    fn name(&self) -> &'static str {
        "nifcloud"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let api_key = required_string(ctx, "api_key", "nifcloud", "API key")?;
        let api_secret = required_string(ctx, "api_secret", "nifcloud", "API secret")?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_nifcloud(&api_key, &api_secret, None)?,
            60,
        )));
        Ok(())
    }
}
