use std::collections::HashMap;
use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct TransipDnsProvider;

impl Provider<DnsContext<'static>> for TransipDnsProvider {
    fn name(&self) -> &'static str {
        "transip"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let login = ctx
            .config
            .get_value("login")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid login for 'transip' DNS provider"
            ))?;

        let private_key_pem = ctx
            .config
            .get_value("private_key_pem")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid private key PEM for 'transip' DNS provider"
            ))?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_transip(&login, &private_key_pem, None)?,
            300,
        )));
        Ok(())
    }
}
