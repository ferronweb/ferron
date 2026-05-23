use std::sync::Arc;

use dns_update::providers::bluecatv2::BluecatV2Config;
use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::{required_string, opt_bool};

pub struct BlueCatV2DnsProvider;

impl Provider<DnsContext<'static>> for BlueCatV2DnsProvider {
    fn name(&self) -> &'static str {
        "bluecatv2"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let config = BluecatV2Config {
            server_url: required_string(ctx, "server_url", "bluecatv2", "server URL")?,
            username: required_string(ctx, "username", "bluecatv2", "username")?,
            password: required_string(ctx, "password", "bluecatv2", "password")?,
            config_name: required_string(ctx, "config_name", "bluecatv2", "config name")?,
            view_name: required_string(ctx, "view_name", "bluecatv2", "view name")?,
            skip_deploy: opt_bool(ctx, "skip_deploy").unwrap_or(false),
            request_timeout: None,
        };

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_bluecatv2(config)?,
            0,
        )));
        Ok(())
    }
}
