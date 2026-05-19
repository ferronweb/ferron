use std::collections::HashMap;
use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct AutoDNSProvider;

impl Provider<DnsContext<'static>> for AutoDNSProvider {
    fn name(&self) -> &'static str {
        "autodns"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let username = ctx
            .config
            .get_value("username")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid username for 'autodns' DNS provider"
            ))?;
        let password = ctx
            .config
            .get_value("password")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid password for 'autodns' DNS provider"
            ))?;
        let context = ctx
            .config
            .get_value("context")
            .and_then(|v| v.as_number())
            .map(|n| n as u32);

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_autodns(&username, &password, context, None)?,
            300,
        )));
        Ok(())
    }
}
