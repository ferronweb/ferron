use std::collections::HashMap;
use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct AlibabaCloudDnsProvider;

impl Provider<DnsContext<'static>> for AlibabaCloudDnsProvider {
    fn name(&self) -> &'static str {
        "alidns"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let access_key_id = ctx
            .config
            .get_value("access_key_id")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid access_key_id for 'alidns' DNS provider"
            ))?;

        let access_key_secret = ctx
            .config
            .get_value("access_key_secret")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid access_key_secret for 'alidns' DNS provider"
            ))?;

        let region = ctx
            .config
            .get_value("region")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()));
        let security_token = ctx
            .config
            .get_value("security_token")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()));
        let line = ctx
            .config
            .get_value("line")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()));

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_alidns(
                &access_key_id,
                &access_key_secret,
                region,
                security_token,
                line,
                None,
            )?,
            600,
        )));
        Ok(())
    }
}
