use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::{opt_string, required_string};

pub struct TencentCloudDnsProvider;

impl Provider<DnsContext<'static>> for TencentCloudDnsProvider {
    fn name(&self) -> &'static str {
        "tencentcloud"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let secret_id = required_string(ctx, "secret_id", "tencentcloud", "secret ID")?;
        let secret_key = required_string(ctx, "secret_key", "tencentcloud", "secret key")?;
        let region = opt_string(ctx, "region");
        let session_token = opt_string(ctx, "session_token");

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_tencentcloud(&secret_id, &secret_key, region, session_token, None)?,
            600,
        )));
        Ok(())
    }
}
