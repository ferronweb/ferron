use std::sync::Arc;

use dns_update::providers::joker::JokerAuth;
use dns_update::DnsUpdater;
use ferron_core::providers::Provider;
use ferron_dns::DnsContext;

use crate::client::DnsStalwartClient;
use crate::providers::util::opt_string;

pub struct JokerDnsProvider;

impl Provider<DnsContext<'static>> for JokerDnsProvider {
    fn name(&self) -> &'static str {
        "joker"
    }

    fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
        let api_key = opt_string(ctx, "api_key");
        let username = opt_string(ctx, "username");
        let password = opt_string(ctx, "password");

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
