use crate::dns_update_common::{dns_update_provider, optional_param, require_param};

dns_update_provider!(
  /// AutoDNS DNS provider
  AutoDnsDnsProvider,
  "AutoDNS",
  |challenge_params| {
    let context = optional_param(challenge_params, "context")
      .map(|v| v.parse::<u32>())
      .transpose()
      .map_err(|e| anyhow::anyhow!("Invalid AutoDNS context: {e}"))?;
    dns_update::DnsUpdater::new_autodns(
      require_param(challenge_params, "username", "AutoDNS username")?,
      require_param(challenge_params, "password", "AutoDNS password")?,
      context,
      None,
    )
  }
);
