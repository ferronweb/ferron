use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::{opt_bool, required_string};

pub struct TransipDnsProvider;

impl Provider<DnsContext<'static>> for TransipDnsProvider {
    fn name(&self) -> &'static str {
        "transip"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let login = required_string(ctx, "login", "transip")?;
        let private_key_pem = required_string(ctx, "private_key_pem", "transip")?;
        let global_key = opt_bool(ctx, "global_key").unwrap_or(false); // Default is from certbot-dns-transip

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_transip(&login, &private_key_pem, global_key, None)?,
            300,
        )));
        Ok(())
    }
}
