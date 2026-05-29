use std::sync::Arc;

use dns_update::providers::route53::Route53Config;
use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::{opt_bool, opt_string, required_string};

pub struct Route53DnsProvider;

impl Provider<DnsContext<'static>> for Route53DnsProvider {
    fn name(&self) -> &'static str {
        "route53"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let config = Route53Config {
            access_key_id: required_string(ctx, "access_key_id", "route53")?,
            secret_access_key: required_string(ctx, "secret_access_key", "route53")?,
            region: opt_string(ctx, "region"),
            session_token: opt_string(ctx, "session_token"),
            hosted_zone_id: opt_string(ctx, "hosted_zone_id"),
            private_zone_only: opt_bool(ctx, "private_zone_only"),
        };

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_route53(config)?,
            1,
        )));
        Ok(())
    }
}
