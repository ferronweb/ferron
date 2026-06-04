use std::sync::Arc;

use dns_update::providers::infoblox::InfobloxConfig;
use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::{opt_string, required_string};

pub struct InfobloxDnsProvider;

impl Provider<DnsContext<'static>> for InfobloxDnsProvider {
    fn name(&self) -> &'static str {
        "infoblox"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let config = InfobloxConfig {
            host: required_string(ctx, "host", "infoblox")?,
            port: opt_string(ctx, "port"),
            username: required_string(ctx, "username", "infoblox")?,
            password: required_string(ctx, "password", "infoblox")?,
            wapi_version: opt_string(ctx, "wapi_version"),
            dns_view: opt_string(ctx, "dns_view"),
            request_timeout: None,
        };

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_infoblox(config)?,
            30,
        )));
        Ok(())
    }
}
