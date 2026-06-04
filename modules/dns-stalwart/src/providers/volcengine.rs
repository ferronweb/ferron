use std::sync::Arc;

use dns_update::providers::volcengine::VolcengineConfig;
use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::{opt_string, required_string};

pub struct VolcengineDnsProvider;

impl Provider<DnsContext<'static>> for VolcengineDnsProvider {
    fn name(&self) -> &'static str {
        "volcengine"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let config = VolcengineConfig {
            access_key: required_string(ctx, "access_key", "volcengine")?,
            secret_key: required_string(ctx, "secret_key", "volcengine")?,
            region: opt_string(ctx, "region"),
            host: opt_string(ctx, "host"),
            scheme: opt_string(ctx, "scheme"),
            request_timeout: None,
        };

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_volcengine(config)?,
            60,
        )));
        Ok(())
    }
}
