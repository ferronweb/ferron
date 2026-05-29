use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::required_string;

pub struct HuaweiCloudDnsProvider;

impl Provider<DnsContext<'static>> for HuaweiCloudDnsProvider {
    fn name(&self) -> &'static str {
        "huaweicloud"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let access_key_id = required_string(ctx, "access_key_id", "huaweicloud")?;
        let access_key_secret = required_string(ctx, "access_key_secret", "huaweicloud")?;
        let region = required_string(ctx, "region", "huaweicloud")?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_huaweicloud(&access_key_id, &access_key_secret, &region, None)?,
            1,
        )));
        Ok(())
    }
}
