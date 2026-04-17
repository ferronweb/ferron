use std::{collections::HashMap, sync::Arc};

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct DnsimpleDnsProvider;

impl Provider<DnsContext<'static>> for DnsimpleDnsProvider {
    fn name(&self) -> &'static str {
        "dnsimple"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let oauth_token = ctx
            .config
            .get_value("oauth_token")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid OAuth token for 'dnsimple' DNS provider"
            ))?;

        let account_id = ctx
            .config
            .get_value("account_id")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid account ID for 'dnsimple' DNS provider"
            ))?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_dnsimple(&oauth_token, &account_id, None)?,
            60,
        )));
        Ok(())
    }
}
