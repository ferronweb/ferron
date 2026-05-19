use std::collections::HashMap;
use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct ScalewayDnsProvider;

impl Provider<DnsContext<'static>> for ScalewayDnsProvider {
    fn name(&self) -> &'static str {
        "scaleway"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let api_token = ctx
            .config
            .get_value("api_token")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid API token for 'scaleway' DNS provider"
            ))?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_scaleway(&api_token, None)?,
            60,
        )));
        Ok(())
    }
}
