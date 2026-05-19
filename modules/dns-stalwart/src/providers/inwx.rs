use std::collections::HashMap;
use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct InwxDnsProvider;

impl Provider<DnsContext<'static>> for InwxDnsProvider {
    fn name(&self) -> &'static str {
        "inwx"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let username = ctx
            .config
            .get_value("username")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid username for 'inwx' DNS provider"
            ))?;

        let password = ctx
            .config
            .get_value("password")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid password for 'inwx' DNS provider"
            ))?;

        let shared_secret = ctx
            .config
            .get_value("shared_secret")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()));

        let sandbox = ctx
            .config
            .get_value("sandbox")
            .and_then(|v| v.as_boolean())
            .unwrap_or(false);

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_inwx(&username, &password, shared_secret, sandbox, None)?,
            300,
        )));
        Ok(())
    }
}
