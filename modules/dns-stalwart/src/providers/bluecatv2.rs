use std::collections::HashMap;
use std::sync::Arc;

use dns_update::providers::bluecatv2::BluecatV2Config;
use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct BlueCatV2DnsProvider;

impl Provider<DnsContext<'static>> for BlueCatV2DnsProvider {
    fn name(&self) -> &'static str {
        "bluecatv2"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let username = ctx
            .config
            .get_value("username")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid username for 'bluecatv2' DNS provider"
            ))?;

        let password = ctx
            .config
            .get_value("password")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid password for 'bluecatv2' DNS provider"
            ))?;

        let server_url = ctx
            .config
            .get_value("server_url")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid server_url for 'bluecatv2' DNS provider"
            ))?;

        let config_name = ctx
            .config
            .get_value("config_name")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid config_name for 'bluecatv2' DNS provider"
            ))?;

        let view_name = ctx
            .config
            .get_value("view_name")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid view_name for 'bluecatv2' DNS provider"
            ))?;

        let skip_deploy = ctx
            .config
            .get_value("skip_deploy")
            .and_then(|v| v.as_boolean())
            .unwrap_or(false);

        let config = BluecatV2Config {
            server_url,
            username,
            password,
            config_name,
            view_name,
            skip_deploy,
            request_timeout: None,
        };

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_bluecatv2(config)?,
            0,
        )));
        Ok(())
    }
}
