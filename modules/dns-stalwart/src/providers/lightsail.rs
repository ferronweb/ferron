use std::collections::HashMap;
use std::sync::Arc;

use dns_update::providers::lightsail::LightsailConfig;
use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct LightsailDnsProvider;

impl Provider<DnsContext<'static>> for LightsailDnsProvider {
    fn name(&self) -> &'static str {
        "lightsail"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let access_key_id = ctx
            .config
            .get_value("access_key_id")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid access key ID for 'lightsail' DNS provider"
            ))?;
        let secret_access_key = ctx
            .config
            .get_value("secret_access_key")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid secret access key for 'lightsail' DNS provider"
            ))?;

        let region = ctx
            .config
            .get_value("region")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()));
        let session_token = ctx
            .config
            .get_value("session_token")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()));
        let domain = ctx
            .config
            .get_value("domain")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()));

        let config = LightsailConfig {
            access_key_id,
            secret_access_key,
            region,
            session_token,
            domain,
            request_timeout: None,
        };

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_lightsail(config)?,
            60,
        )));
        Ok(())
    }
}
