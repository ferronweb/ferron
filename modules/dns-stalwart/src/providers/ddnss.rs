use std::collections::HashMap;
use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct DDNSSProvider;

impl Provider<DnsContext<'static>> for DDNSSProvider {
    fn name(&self) -> &'static str {
        "ddnss"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let key = ctx
            .config
            .get_value("key")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid key for 'ddnss' DNS provider"
            ))?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_ddnss(&key, None)?,
            900,
        )));
        Ok(())
    }
}
