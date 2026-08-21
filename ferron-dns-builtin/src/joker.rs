use crate::dns_update_common::{dns_update_provider, optional_param};

dns_update_provider!(
  /// Joker.com DNS provider
  JokerDnsProvider,
  "Joker.com",
  |challenge_params| {
    let auth = if let (Some(username), Some(password)) = (
      optional_param(challenge_params, "username"),
      optional_param(challenge_params, "password"),
    ) {
      dns_update::providers::joker::JokerAuth::username_password(username, password)
    } else if let Some(api_key) = optional_param(challenge_params, "api_key") {
      dns_update::providers::joker::JokerAuth::api_key(api_key)
    } else {
      Err(anyhow::anyhow!("Missing Joker.com API key or username and password"))?
    };
    dns_update::DnsUpdater::new_joker(auth, None)
  }
);
