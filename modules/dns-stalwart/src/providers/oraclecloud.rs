use std::sync::Arc;

use dns_update::providers::oraclecloud::OracleCloudConfig;
use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::{opt_string, required_string};

pub struct OracleCloudDnsProvider;

impl Provider<DnsContext<'static>> for OracleCloudDnsProvider {
    fn name(&self) -> &'static str {
        "oraclecloud"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let config = OracleCloudConfig {
            tenancy_ocid: required_string(ctx, "tenancy_ocid", "oraclecloud")?,
            user_ocid: required_string(ctx, "user_ocid", "oraclecloud")?,
            fingerprint: required_string(ctx, "fingerprint", "oraclecloud")?,
            private_key_pem: required_string(ctx, "private_key_pem", "oraclecloud")?,
            private_key_password: opt_string(ctx, "private_key_password"),
            region: required_string(ctx, "region", "oraclecloud")?,
            compartment_ocid: required_string(ctx, "compartment_ocid", "oraclecloud")?,
            request_timeout: None,
        };

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_oraclecloud(config)?,
            30,
        )));
        Ok(())
    }
}
