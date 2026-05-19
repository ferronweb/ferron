use std::collections::HashMap;
use std::sync::Arc;

use dns_update::providers::infoblox::InfobloxConfig;
use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct InfobloxDnsProvider;

impl Provider<DnsContext<'static>> for InfobloxDnsProvider {
    fn name(&self) -> &'static str {
        "infoblox"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let host = ctx
            .config
            .get_value("host")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid host for 'infoblox' DNS provider"
            ))?;

        let port = ctx
            .config
            .get_value("port")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()));

        let username = ctx
            .config
            .get_value("username")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid username for 'infoblox' DNS provider"
            ))?;

        let password = ctx
            .config
            .get_value("password")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid password for 'infoblox' DNS provider"
            ))?;

        let wapi_version = ctx
            .config
            .get_value("wapi_version")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()));

        let dns_view = ctx
            .config
            .get_value("dns_view")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()));

        let config = InfobloxConfig {
            host,
            port,
            username,
            password,
            wapi_version,
            dns_view,
            request_timeout: None,
        };
        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_infoblox(config)?,
            30,
        )));
        Ok(())
    }
}
