use std::collections::HashMap;
use std::sync::Arc;

use dns_update::providers::oraclecloud::OracleCloudConfig;
use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct OracleCloudDnsProvider;

impl Provider<DnsContext<'static>> for OracleCloudDnsProvider {
    fn name(&self) -> &'static str {
        "oraclecloud"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let tenancy_ocid = ctx
            .config
            .get_value("tenancy_ocid")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid tenancy OCID for 'oraclecloud' DNS provider"
            ))?;

        let user_ocid = ctx
            .config
            .get_value("user_ocid")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid user OCID for 'oraclecloud' DNS provider"
            ))?;

        let compartment_ocid = ctx
            .config
            .get_value("compartment_ocid")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid compartment OCID for 'oraclecloud' DNS provider"
            ))?;

        let region = ctx
            .config
            .get_value("region")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid region for 'oraclecloud' DNS provider"
            ))?;

        let fingerprint = ctx
            .config
            .get_value("fingerprint")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid fingerprint for 'oraclecloud' DNS provider"
            ))?;

        let private_key_pem = ctx
            .config
            .get_value("private_key_pem")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid private key PEM for 'oraclecloud' DNS provider"
            ))?;

        let private_key_password = ctx
            .config
            .get_value("private_key_password")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()));

        let config = OracleCloudConfig {
            tenancy_ocid,
            user_ocid,
            fingerprint,
            private_key_pem,
            private_key_password,
            compartment_ocid,
            region,
            request_timeout: None,
        };

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_oraclecloud(config)?,
            30,
        )));
        Ok(())
    }
}
