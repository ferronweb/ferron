use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// Hurricane Electric DNS provider
  HurricaneDnsProvider,
  "Hurricane Electric",
  |challenge_params| {
    let credentials = require_param(challenge_params, "credentials", "Hurricane Electric credentials")?;
    let mut credentials_map = std::collections::HashMap::new();
    for pair in credentials.split(',') {
      let (key, value) = pair
        .split_once('=')
        .ok_or_else(|| anyhow::anyhow!("Invalid Hurricane Electric credentials format"))?;
      credentials_map.insert(key.to_string(), value.to_string());
    }
    dns_update::DnsUpdater::new_hurricane(credentials_map, None)
  }
);
