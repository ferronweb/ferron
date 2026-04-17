use std::{collections::HashMap, sync::Arc};

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct CloudflareDnsProvider;

impl Provider<DnsContext<'static>> for CloudflareDnsProvider {
    fn name(&self) -> &'static str {
        "cloudflare"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let api_key = ctx
            .config
            .get_value("api_key")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid API key for 'cloudflare' DNS provider"
            ))?;

        let email = ctx
            .config
            .get_value("email")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()));

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_cloudflare(&api_key, email.as_deref(), None)?,
            60,
        )));
        Ok(())
    }
}
