use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::required_string;

pub struct BaiduCloudDnsProvider;

impl Provider<DnsContext<'static>> for BaiduCloudDnsProvider {
    fn name(&self) -> &'static str {
        "baiducloud"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let access_key_id = required_string(ctx, "access_key_id", "baiducloud")?;
        let access_key_secret = required_string(ctx, "access_key_secret", "baiducloud")?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_baiducloud(&access_key_id, &access_key_secret, None)?,
            300,
        )));
        Ok(())
    }
}
