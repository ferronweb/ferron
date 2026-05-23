use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::{opt_bool, opt_string, required_string};

pub struct InwxDnsProvider;

impl Provider<DnsContext<'static>> for InwxDnsProvider {
    fn name(&self) -> &'static str {
        "inwx"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let username = required_string(ctx, "username", "inwx", "username")?;
        let password = required_string(ctx, "password", "inwx", "password")?;
        let shared_secret = opt_string(ctx, "shared_secret");
        let sandbox = opt_bool(ctx, "sandbox").unwrap_or(false);

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_inwx(&username, &password, shared_secret, sandbox, None)?,
            300,
        )));
        Ok(())
    }
}
