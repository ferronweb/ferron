use std::sync::Arc;

use dns_update::providers::lightsail::LightsailConfig;
use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::{opt_string, required_string};

pub struct LightsailDnsProvider;

impl Provider<DnsContext<'static>> for LightsailDnsProvider {
    fn name(&self) -> &'static str {
        "lightsail"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let config = LightsailConfig {
            access_key_id: required_string(ctx, "access_key_id", "lightsail", "access key ID")?,
            secret_access_key: required_string(
                ctx,
                "secret_access_key",
                "lightsail",
                "secret access key",
            )?,
            region: opt_string(ctx, "region"),
            session_token: opt_string(ctx, "session_token"),
            domain: opt_string(ctx, "domain"),
            request_timeout: None,
        };

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_lightsail(config)?,
            60,
        )));
        Ok(())
    }
}
