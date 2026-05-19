use std::collections::HashMap;
use std::sync::Arc;

use dns_update::providers::edgedns::EdgeDnsConfig;
use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct EdgeDnsProvider;

impl Provider<DnsContext<'static>> for EdgeDnsProvider {
    fn name(&self) -> &'static str {
        "edgedns"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let host = ctx
            .config
            .get_value("host")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid host for 'edgedns' DNS provider"
            ))?;

        let client_token = ctx
            .config
            .get_value("client_token")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid client token for 'edgedns' DNS provider"
            ))?;

        let client_secret = ctx
            .config
            .get_value("client_secret")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid client secret for 'edgedns' DNS provider"
            ))?;

        let access_token = ctx
            .config
            .get_value("access_token")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid access token for 'edgedns' DNS provider"
            ))?;

        let account_switch_key = ctx
            .config
            .get_value("account_switch_key")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()));

        let config = EdgeDnsConfig {
            host,
            client_token,
            client_secret,
            access_token,
            account_switch_key,
            request_timeout: None,
        };

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_edgedns(config)?,
            300,
        )));
        Ok(())
    }
}
