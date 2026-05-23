use std::sync::Arc;

use dns_update::providers::edgedns::EdgeDnsConfig;
use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::{opt_string, required_string};

pub struct EdgeDnsProvider;

impl Provider<DnsContext<'static>> for EdgeDnsProvider {
    fn name(&self) -> &'static str {
        "edgedns"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let config = EdgeDnsConfig {
            host: required_string(ctx, "host", "edgedns", "host")?,
            client_token: required_string(ctx, "client_token", "edgedns", "client token")?,
            client_secret: required_string(ctx, "client_secret", "edgedns", "client secret")?,
            access_token: required_string(ctx, "access_token", "edgedns", "access token")?,
            account_switch_key: opt_string(ctx, "account_switch_key"),
            request_timeout: None,
        };

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_edgedns(config)?,
            300,
        )));
        Ok(())
    }
}
