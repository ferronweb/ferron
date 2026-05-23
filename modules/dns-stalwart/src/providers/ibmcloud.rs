use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::required_string;

pub struct IbmCloudDnsProvider;

impl Provider<DnsContext<'static>> for IbmCloudDnsProvider {
    fn name(&self) -> &'static str {
        "ibmcloud"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let username = required_string(ctx, "username", "ibmcloud", "username")?;
        let api_key = required_string(ctx, "api_key", "ibmcloud", "API key")?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_ibmcloud(&username, &api_key, None)?,
            60,
        )));
        Ok(())
    }
}
