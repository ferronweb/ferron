use std::collections::HashMap;
use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct SafeDNSProvider;

impl Provider<DnsContext<'static>> for SafeDNSProvider {
    fn name(&self) -> &'static str {
        "safedns"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let auth_token = ctx
            .config
            .get_value("auth_token")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid auth token for 'safedns' DNS provider"
            ))?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_safedns(&auth_token, None)?,
            300,
        )));
        Ok(())
    }
}
