use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::{opt_string, required_string};

pub struct AlibabaCloudDnsProvider;

impl Provider<DnsContext<'static>> for AlibabaCloudDnsProvider {
    fn name(&self) -> &'static str {
        "alidns"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let access_key_id = required_string(ctx, "access_key_id", "alidns", "access key ID")?;
        let access_key_secret =
            required_string(ctx, "access_key_secret", "alidns", "access key secret")?;
        let region = opt_string(ctx, "region");
        let security_token = opt_string(ctx, "security_token");
        let line = opt_string(ctx, "line");

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_alidns(
                &access_key_id,
                &access_key_secret,
                region,
                security_token,
                line,
                None,
            )?,
            600,
        )));
        Ok(())
    }
}
