use std::collections::HashMap;
use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct ClouDNSProvider;

impl Provider<DnsContext<'static>> for ClouDNSProvider {
    fn name(&self) -> &'static str {
        "cloudns"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let auth_id = ctx
            .config
            .get_value("auth_id")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()));
        let sub_auth_id = ctx
            .config
            .get_value("sub_auth_id")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()));

        let password = ctx
            .config
            .get_value("password")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid password for 'cloudns' DNS provider"
            ))?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_cloudns(auth_id, sub_auth_id, &password, None)?,
            60,
        )));
        Ok(())
    }
}
