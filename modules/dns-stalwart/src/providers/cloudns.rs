use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::{opt_string, required_string};

pub struct ClouDNSProvider;

impl Provider<DnsContext<'static>> for ClouDNSProvider {
    fn name(&self) -> &'static str {
        "cloudns"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let auth_id = opt_string(ctx, "auth_id");
        let sub_auth_id = opt_string(ctx, "sub_auth_id");
        let password = required_string(ctx, "password", "cloudns", "password")?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_cloudns(auth_id, sub_auth_id, &password, None)?,
            60,
        )));
        Ok(())
    }
}
