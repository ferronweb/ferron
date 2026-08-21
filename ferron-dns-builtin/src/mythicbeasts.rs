use crate::dns_update_common::{dns_update_provider, require_param};

dns_update_provider!(
  /// Mythic Beasts DNS provider
  MythicBeastsDnsProvider,
  "Mythic Beasts",
  |challenge_params| dns_update::DnsUpdater::new_mythicbeasts(
    require_param(challenge_params, "username", "Mythic Beasts API username")?,
    require_param(challenge_params, "password", "Mythic Beasts API password")?,
    None,
  )
);
