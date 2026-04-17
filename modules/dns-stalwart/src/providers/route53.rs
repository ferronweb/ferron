use std::{collections::HashMap, sync::Arc};

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct Route53DnsProvider;

impl Provider<DnsContext<'static>> for Route53DnsProvider {
    fn name(&self) -> &'static str {
        "route53"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let access_key_id = ctx
            .config
            .get_value("access_key_id")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid access key ID for 'route53' DNS provider"
            ))?;
        let secret_access_key = ctx
            .config
            .get_value("secret_access_key")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid secret access key for 'route53' DNS provider"
            ))?;

        let region = ctx
            .config
            .get_value("region")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()));
        let session_token = ctx
            .config
            .get_value("session_token")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()));
        let hosted_zone_id = ctx
            .config
            .get_value("hosted_zone_id")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()));
        let private_zone_only = ctx
            .config
            .get_value("private_zone_only")
            .and_then(|v| v.as_boolean());

        let config = dns_update::providers::route53::Route53Config {
            access_key_id,
            secret_access_key,
            region,
            session_token,
            hosted_zone_id,
            private_zone_only,
        };

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_route53(config)?,
            1,
        )));
        Ok(())
    }
}
