use std::collections::HashMap;
use std::sync::Arc;

use dns_update::providers::volcengine::VolcengineConfig;
use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct VolcengineDnsProvider;

impl Provider<DnsContext<'static>> for VolcengineDnsProvider {
    fn name(&self) -> &'static str {
        "volcengine"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let access_key = ctx
            .config
            .get_value("access_key")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid access_key for 'volcengine' DNS provider"
            ))?;

        let secret_key = ctx
            .config
            .get_value("secret_key")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid secret_key for 'volcengine' DNS provider"
            ))?;

        let region = ctx
            .config
            .get_value("region")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()));
        let host = ctx
            .config
            .get_value("host")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()));
        let scheme = ctx
            .config
            .get_value("scheme")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()));

        let config = VolcengineConfig {
            access_key,
            secret_key,
            region,
            host,
            scheme,
            request_timeout: None,
        };

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_volcengine(config)?,
            60,
        )));
        Ok(())
    }
}
