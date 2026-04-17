use std::{collections::HashMap, sync::Arc};

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct PorkbunDnsProvider;

impl Provider<DnsContext<'static>> for PorkbunDnsProvider {
    fn name(&self) -> &'static str {
        "porkbun"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let api_key = ctx
            .config
            .get_value("api_key")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid API key for 'porkbun' DNS provider"
            ))?;

        let secret_key = ctx
            .config
            .get_value("secret_key")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid secret key for 'porkbun' DNS provider"
            ))?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_porkbun(&api_key, &secret_key, None)?,
            600,
        )));
        Ok(())
    }
}
