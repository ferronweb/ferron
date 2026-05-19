use std::collections::HashMap;
use std::sync::Arc;

use dns_update::providers::joker::JokerAuth;
use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;

pub struct JokerDnsProvider;

impl Provider<DnsContext<'static>> for JokerDnsProvider {
    fn name(&self) -> &'static str {
        "joker"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let api_key = ctx
            .config
            .get_value("api_key")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()));
        let username = ctx
            .config
            .get_value("username")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()));
        let password = ctx
            .config
            .get_value("password")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()));

        let auth = if let (Some(username), Some(password)) = (username, password) {
            JokerAuth::UsernamePassword { username, password }
        } else if let Some(api_key) = api_key {
            JokerAuth::ApiKey(api_key)
        } else {
            return Err("No API key or username/password provided for 'joker' DNS provider".into());
        };

        ctx.client = Some(Arc::new(DnsStalwartClient::new(
            DnsUpdater::new_joker(auth, None)?,
            300,
        )));
        Ok(())
    }
}
