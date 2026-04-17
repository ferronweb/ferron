use std::{collections::HashMap, sync::Arc};

use dns_update::{providers::ovh::OvhEndpoint, DnsUpdater};
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct OvhDnsProvider;

impl Provider<DnsContext<'static>> for OvhDnsProvider {
    fn name(&self) -> &'static str {
        "ovh"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let application_key = ctx
            .config
            .get_value("application_key")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid application key for 'ovh' DNS provider"
            ))?;

        let application_secret = ctx
            .config
            .get_value("application_secret")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid application secret for 'ovh' DNS provider"
            ))?;

        let consumer_key = ctx
            .config
            .get_value("consumer_key")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid consumer key for 'ovh' DNS provider"
            ))?;

        let endpoint_name = ctx
            .config
            .get_value("endpoint")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .ok_or(anyhow::anyhow!(
                "Missing or invalid endpoint for 'ovh' DNS provider"
            ))?;

        let endpoint = match endpoint_name.as_str() {
            "ovh-eu" => OvhEndpoint::OvhEu,
            "ovh-ca" => OvhEndpoint::OvhCa,
            "kimsufi-eu" => OvhEndpoint::KimsufiEu,
            "kimsufi-ca" => OvhEndpoint::KimsufiCa,
            "soyoustart-eu" => OvhEndpoint::SoyoustartCa,
            "soyoustart-ca" => OvhEndpoint::SoyoustartEu,
            _ => Err(anyhow::anyhow!("Invalid OVH endpoint name"))?,
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
