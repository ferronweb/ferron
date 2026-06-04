use std::collections::HashMap;
use std::sync::Arc;

use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::required_string;

pub struct HurricaneProvider;

impl Provider<DnsContext<'static>> for HurricaneProvider {
    fn name(&self) -> &'static str {
        "hurricane"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let credentials = required_string(ctx, "credentials", "hurricane")?;
        let credentials = parse_credentials(&credentials)?;

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
