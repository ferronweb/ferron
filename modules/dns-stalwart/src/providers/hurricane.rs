use std::collections::HashMap;
use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct HurricaneProvider;

impl Provider<DnsContext<'static>> for HurricaneProvider {
    fn name(&self) -> &'static str {
        "hurricane"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let credentials = ctx
            .config
            .get_value("credentials")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .and_then(|v| parse_credentials(&v).ok())
            .ok_or(anyhow::anyhow!(
                "Missing or invalid credentials for 'hurricane' DNS provider"
            ))?;

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_hurricane(credentials, None)?,
            300,
        )));
        Ok(())
    }
}

fn parse_credentials(credentials: &str) -> Result<HashMap<String, String>, anyhow::Error> {
    let mut result = HashMap::new();
    for pair in credentials.split(',') {
        let (key, value) = pair
            .split_once('=')
            .ok_or(anyhow::anyhow!("Invalid credentials format"))?;
        result.insert(key.to_string(), value.to_string());
    }
    Ok(result)
}
