use std::collections::HashMap;
use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct NetcupDnsProvider;

impl Provider<DnsContext<'static>> for NetcupDnsProvider {
    fn name(&self) -> &'static str {
        "netcup"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let customer_number = ctx
            .config
            .get_value("customer_number")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid customer number for 'netcup' DNS provider"
            ))?;

        let api_key = ctx
            .config
            .get_value("api_key")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid API key for 'netcup' DNS provider"
            ))?;

        let api_password = ctx
            .config
            .get_value("api_password")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid API password for 'netcup' DNS provider"
            ))?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_netcup(&customer_number, &api_key, &api_password, None)?,
            60,
        )));
        Ok(())
    }
}
