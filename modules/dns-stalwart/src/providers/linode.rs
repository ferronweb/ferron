use std::collections::HashMap;
use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct LinodeDnsProvider;

impl Provider<DnsContext<'static>> for LinodeDnsProvider {
    fn name(&self) -> &'static str {
        "linode"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let auth_token = ctx
            .config
            .get_value("auth_token")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid auth token for 'linode' DNS provider"
            ))?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_linode(&auth_token, None)?,
            300,
        )));
        Ok(())
    }
}
