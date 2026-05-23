use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::required_string;

pub struct TransipDnsProvider;

impl Provider<DnsContext<'static>> for TransipDnsProvider {
    fn name(&self) -> &'static str {
        "transip"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let login = required_string(ctx, "login", "transip", "login")?;
        let private_key_pem =
            required_string(ctx, "private_key_pem", "transip", "private key PEM")?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_transip(&login, &private_key_pem, None)?,
            300,
        )));
        Ok(())
    }
}
