use std::sync::Arc;

use dns_update::{providers::ovh::OvhEndpoint, DnsUpdater};
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::required_string;

pub struct OvhDnsProvider;

impl Provider<DnsContext<'static>> for OvhDnsProvider {
    fn name(&self) -> &'static str {
        "ovh"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let application_key = required_string(ctx, "application_key", "ovh", "application key")?;
        let application_secret =
            required_string(ctx, "application_secret", "ovh", "application secret")?;
        let consumer_key = required_string(ctx, "consumer_key", "ovh", "consumer key")?;

        let endpoint_name = required_string(ctx, "endpoint", "ovh", "endpoint")?;
        let endpoint = match endpoint_name.as_str() {
            "ovh-eu" => OvhEndpoint::OvhEu,
            "ovh-ca" => OvhEndpoint::OvhCa,
            "kimsufi-eu" => OvhEndpoint::KimsufiEu,
            "kimsufi-ca" => OvhEndpoint::KimsufiCa,
            "soyoustart-eu" => OvhEndpoint::SoyoustartEu,
            "soyoustart-ca" => OvhEndpoint::SoyoustartCa,
            _ => Err(anyhow::anyhow!(
                "Invalid OVH endpoint name for 'ovh' DNS provider"
            ))?,
        };

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_ovh(
                &application_key,
                &application_secret,
                &consumer_key,
                endpoint,
                None,
            )?,
            60,
        )));
        Ok(())
    }
}
