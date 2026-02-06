//! TLS and ACME configuration builder utilities.
//!
//! This module is responsible for translating server configuration entries
//! into concrete TLS listener state, SNI resolvers, and ACME configurations.
//!
//! Responsibilities include:
//! - Manual TLS certificate loading
//! - Automatic TLS (ACME) configuration
//! - On-demand vs eager ACME flows
//! - Resolver wiring per TLS port
//!
//! This module is intentionally side-effectful and mutates `TlsBuildContext`
//! as part of the build process.

use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::Arc;

use async_channel::{Receiver, Sender};
use ferron_common::config::ServerConfigurationFilters;
use ferron_common::get_entry;
use ferron_common::logging::LogMessage;
use instant_acme::ChallengeType;
use rustls::crypto::CryptoProvider;
use rustls::server::ResolvesServerCert;
use rustls::sign::CertifiedKey;
use tokio::sync::RwLock;

use crate::acme::{AcmeCache, AcmeConfig, AcmeOnDemandConfig, Http01DataLock, TlsAlpn01DataLock, TlsAlpn01Resolver};
use crate::util::{load_certs, load_private_key, CustomSniResolver, OneCertifiedKeyResolver};

/// Accumulates TLS and ACME-related state while building listener configuration.
///
/// This struct is mutated during server configuration processing and later
/// consumed by the runtime to:
/// - Spawn TLS listeners
/// - Preload certificates
/// - Run ACME background tasks
/// - Handle on-demand certificate issuance
///
/// It intentionally groups multiple maps and locks to avoid threading a large
/// number of parameters through builder functions.
pub struct TlsBuildContext {
  pub tls_ports: HashMap<u16, CustomSniResolver>,
  #[allow(clippy::type_complexity)]
  pub tls_port_locks: HashMap<u16, Arc<RwLock<Vec<(String, Arc<dyn ResolvesServerCert>)>>>>,
  pub nonencrypted_ports: HashSet<u16>,
  pub certified_keys_to_preload: HashMap<u16, Vec<Arc<CertifiedKey>>>,
  pub used_sni_hostnames: HashSet<(u16, Option<String>)>,
  pub automatic_tls_used_sni_hostnames: HashSet<(u16, Option<String>)>,
  pub acme_tls_alpn_01_resolvers: HashMap<u16, TlsAlpn01Resolver>,
  pub acme_tls_alpn_01_resolver_locks: HashMap<u16, Arc<RwLock<Vec<TlsAlpn01DataLock>>>>,
  pub acme_http_01_resolvers: Arc<RwLock<Vec<Http01DataLock>>>,
  pub acme_configs: Vec<AcmeConfig>,
  pub acme_on_demand_configs: Vec<AcmeOnDemandConfig>,
  pub acme_on_demand_tx: Sender<(String, u16)>,
  pub acme_on_demand_rx: Receiver<(String, u16)>,
}

impl Default for TlsBuildContext {
  fn default() -> Self {
    let (acme_on_demand_tx, acme_on_demand_rx) = async_channel::unbounded();
    Self {
      tls_ports: HashMap::new(),
      tls_port_locks: HashMap::new(),
      nonencrypted_ports: HashSet::new(),
      certified_keys_to_preload: HashMap::new(),
      used_sni_hostnames: HashSet::new(),
      automatic_tls_used_sni_hostnames: HashSet::new(),
      acme_tls_alpn_01_resolvers: HashMap::new(),
      acme_tls_alpn_01_resolver_locks: HashMap::new(),
      acme_http_01_resolvers: Arc::new(RwLock::new(Vec::new())),
      acme_configs: Vec::new(),
      acme_on_demand_configs: Vec::new(),
      acme_on_demand_tx,
      acme_on_demand_rx,
    }
  }
}

/// Reads the default port from the given server configuration.
pub fn read_default_port(config: Option<&ferron_common::config::ServerConfiguration>, is_https: bool) -> Option<u16> {
  let fallback = if is_https { 443 } else { 80 };
  config
    .and_then(|c| {
      if is_https {
        get_entry!("default_https_port", c)
      } else {
        get_entry!("default_http_port", c)
      }
    })
    .and_then(|e| e.values.first())
    .map_or(Some(fallback), |v| {
      if v.is_null() {
        None
      } else {
        Some(v.as_i128().unwrap_or(fallback as i128) as u16)
      }
    })
}

/// Resolves the SNI hostname from the given filters.
pub fn resolve_sni_hostname(filters: &ServerConfigurationFilters) -> Option<String> {
  filters.hostname.clone().or_else(|| {
    if filters.ip.is_some_and(|ip| ip.is_loopback()) {
      // Host blocks with "localhost" specified will have "localhost" SNI hostname
      return Some("localhost".to_string());
    }

    // !!! UNTESTED, many clients don't send SNI hostname when accessing via IP address anyway
    match filters.ip {
      Some(IpAddr::V4(addr)) => Some(addr.to_string()),
      Some(IpAddr::V6(addr)) => Some(format!("[{addr}]")),
      None => None,
    }
  })
}

/// Ensures that a TLS SNI resolver exists for the given port.
///
/// If the resolver does not already exist, it is created along with its
/// associated resolver lock and inserted into the context.
///
/// Returns a mutable reference to the resolver for further configuration.
fn ensure_tls_port_resolver(ctx: &mut TlsBuildContext, port: u16) -> &mut CustomSniResolver {
  ctx.tls_ports.entry(port).or_insert_with(|| {
    let list = Arc::new(RwLock::new(Vec::new()));
    ctx.tls_port_locks.insert(port, list.clone());
    CustomSniResolver::with_resolvers(list)
  })
}

/// Configures a manually provided TLS certificate and private key.
///
/// This function:
/// - Loads and validates the certificate and private key
/// - Registers the certificate for preloading
/// - Installs an SNI resolver (or fallback resolver) for the given port
///
/// Manual TLS always takes precedence over automatic TLS.
pub fn handle_manual_tls(
  ctx: &mut TlsBuildContext,
  crypto_provider: &CryptoProvider,
  port: u16,
  sni_hostname: Option<String>,
  cert_path: &str,
  key_path: &str,
) -> anyhow::Result<()> {
  let certs = load_certs(cert_path).map_err(|e| anyhow::anyhow!("Cannot load certificate {cert_path}: {e}"))?;

  let key = load_private_key(key_path).map_err(|e| anyhow::anyhow!("Cannot load key {key_path}: {e}"))?;

  let signing_key = crypto_provider
    .key_provider
    .load_private_key(key)
    .map_err(|e| anyhow::anyhow!("Invalid private key {key_path}: {e}"))?;

  let certified_key = Arc::new(CertifiedKey::new(certs, signing_key));

  ctx
    .certified_keys_to_preload
    .entry(port)
    .or_default()
    .push(certified_key.clone());

  let resolver = Arc::new(OneCertifiedKeyResolver::new(certified_key));
  let sni_resolver = ensure_tls_port_resolver(ctx, port);

  match &sni_hostname {
    Some(host) => sni_resolver.load_host_resolver(host, resolver),
    None => sni_resolver.load_fallback_resolver(resolver),
  }

  ctx.used_sni_hostnames.insert((port, sni_hostname));
  Ok(())
}

/// Parses ACME challenge type from server configuration.
fn parse_challenge_type(
  server: &ferron_common::config::ServerConfiguration,
) -> anyhow::Result<(ChallengeType, HashMap<String, String>)> {
  let entry = get_entry!("auto_tls_challenge", server);

  let ty = entry
    .and_then(|e| e.values.first())
    .and_then(|v| v.as_str())
    .unwrap_or("tls-alpn-01")
    .to_uppercase();

  let params = entry
    .map(|e| {
      e.props
        .iter()
        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
        .collect()
    })
    .unwrap_or_default();

  let challenge = match ty.as_str() {
    "HTTP-01" => ChallengeType::Http01,
    "TLS-ALPN-01" => ChallengeType::TlsAlpn01,
    "DNS-01" => ChallengeType::Dns01,
    _ => anyhow::bail!("Unsupported ACME challenge type: {ty}"),
  };

  Ok((challenge, params))
}

/// Checks if the server should be skipped.
pub fn should_skip_server(server: &ferron_common::config::ServerConfiguration) -> bool {
  server.filters.is_global_non_host() || (server.filters.is_global() && server.entries.is_empty())
}

/// Obtains the certificate and key for a manual TLS entry in server configuration.
pub fn manual_tls_entry(server: &ferron_common::config::ServerConfiguration) -> Option<(&str, &str)> {
  let tls_entry = get_entry!("tls", server)?;

  if tls_entry.values.len() != 2 {
    return None;
  }

  let cert = tls_entry.values[0].as_str()?;
  let key = tls_entry.values[1].as_str()?;

  Some((cert, key))
}

/// Handles non-encrypted ports for a server configuration.
pub fn handle_nonencrypted_ports(
  ctx: &mut TlsBuildContext,
  server: &ferron_common::config::ServerConfiguration,
  default_http_port: Option<u16>,
) {
  // If TLS is explicitly configured, don't add HTTP
  if get_entry!("tls", server).is_some() {
    return;
  }

  // If automatic TLS is explicitly enabled, HTTP is usually disabled
  if get_entry!("auto_tls", server)
    .and_then(|e| e.values.first())
    .and_then(|v| v.as_bool())
    .unwrap_or(false)
  {
    return;
  }

  if let Some(port) = server.filters.port.or(default_http_port) {
    ctx.nonencrypted_ports.insert(port);
  }
}

/// Configures automatic TLS (ACME) for a server configuration.
///
/// Depending on the server configuration, this function may:
/// - Configure eager (startup-time) ACME
/// - Configure on-demand ACME
/// - Skip automatic TLS if required conditions are not met
///
/// This function does not perform ACME issuance itself; it only wires the
/// required resolvers and configuration objects.
pub fn handle_automatic_tls(
  ctx: &mut TlsBuildContext,
  server: &ferron_common::config::ServerConfiguration,
  port: u16,
  sni_hostname: Option<String>,
  crypto_provider: Arc<CryptoProvider>,
  memory_acme_account_cache_data: Arc<RwLock<HashMap<String, Vec<u8>>>>,
) -> anyhow::Result<Option<LogMessage>> {
  let on_demand = get_entry!("auto_tls_on_demand", server)
    .and_then(|e| e.values.first())
    .and_then(|v| v.as_bool())
    .unwrap_or(false);

  // Reject IP-based identifiers
  if let Some(host) = &sni_hostname {
    if host.parse::<IpAddr>().is_ok() {
      return Ok(Some(LogMessage::new(
        format!(
          "Ferron's automatic TLS functionality doesn't support IP address-based identifiers, \
            skipping SNI host \"{host}\"..."
        ),
        true,
      )));
    }
  }

  // Automatic TLS requires SNI unless global
  if sni_hostname.is_none() && !server.filters.is_global() && !server.filters.is_global_non_host() {
    return Ok(Some(LogMessage::new(
      "Skipping automatic TLS for a host without a SNI hostname...".to_string(),
      true,
    )));
  }

  let (challenge_type, challenge_params) = parse_challenge_type(server)?;

  if let Some(sni_hostname) = &sni_hostname {
    let is_wildcard_domain = sni_hostname.starts_with("*.");
    if is_wildcard_domain && !on_demand {
      match &challenge_type {
        ChallengeType::Http01 => {
          return Ok(Some(LogMessage::new(
            format!(
              "HTTP-01 ACME challenge doesn't support wildcard hostnames, skipping SNI host \"{sni_hostname}\"..."
            ),
            true,
          )));
        }
        ChallengeType::TlsAlpn01 => {
          return Ok(Some(LogMessage::new(
            format!(
              "TLS-ALPN-01 ACME challenge doesn't support wildcard hostnames, skipping SNI host \"{sni_hostname}\"..."
            ),
            true,
          )));
        }
        _ => (),
      }
    }
  }

  // DNS provider only applies to DNS-01
  let dns_provider = if challenge_type == ChallengeType::Dns01 {
    let provider = challenge_params
      .get("provider")
      .ok_or_else(|| anyhow::anyhow!("DNS-01 challenge requires a provider"))?;
    Some(ferron_load_modules::get_dns_provider(provider, &challenge_params).map_err(|e| anyhow::anyhow!(e))?)
  } else {
    None
  };

  if on_demand {
    build_on_demand_acme(
      ctx,
      server,
      port,
      sni_hostname,
      challenge_type,
      dns_provider,
      crypto_provider,
    )?;
  } else if let Some(host) = sni_hostname {
    build_eager_acme(
      ctx,
      server,
      port,
      host,
      challenge_type,
      dns_provider,
      crypto_provider,
      memory_acme_account_cache_data,
    )?;
  }

  Ok(None)
}

/// Builds an on-demand ACME configuration.
///
/// On-demand ACME defers certificate issuance until a client connects and
/// requests a hostname that does not yet have a certificate. This is typically
/// used for wildcard or dynamic hostnames.
fn build_on_demand_acme(
  ctx: &mut TlsBuildContext,
  server: &ferron_common::config::ServerConfiguration,
  port: u16,
  sni_hostname: Option<String>,
  challenge_type: ChallengeType,
  dns_provider: Option<Arc<dyn ferron_common::dns::DnsProvider + Send + Sync>>,
  crypto_provider: Arc<CryptoProvider>,
) -> anyhow::Result<()> {
  // TLS-ALPN-01 requires a dedicated resolver
  if challenge_type == ChallengeType::TlsAlpn01 {
    let resolver_list = Arc::new(RwLock::new(Vec::new()));
    ctx.acme_tls_alpn_01_resolver_locks.insert(port, resolver_list.clone());

    ctx
      .acme_tls_alpn_01_resolvers
      .insert(port, TlsAlpn01Resolver::with_resolvers(resolver_list));
  }

  // Install fallback sender into SNI resolver
  let fallback_sender = ctx.acme_on_demand_tx.clone();
  let sni_resolver = ensure_tls_port_resolver(ctx, port);
  sni_resolver.load_fallback_sender(fallback_sender, port);

  let rustls_client_config =
    super::acme::build_rustls_client_config(server, crypto_provider).map_err(|e| anyhow::anyhow!(e))?;

  let config = AcmeOnDemandConfig {
    rustls_client_config,
    challenge_type,
    contact: get_entry!("auto_tls_contact", server)
      .and_then(|e| e.values.first())
      .and_then(|v| v.as_str())
      .map(|c| vec![format!("mailto:{c}")])
      .unwrap_or_default(),
    directory: super::acme::resolve_acme_directory(server),
    eab_key: super::acme::parse_eab(server)?,
    profile: get_entry!("auto_tls_profile", server)
      .and_then(|e| e.values.first())
      .and_then(|v| v.as_str())
      .map(str::to_string),
    cache_path: super::acme::resolve_acme_cache_path(server)?,
    sni_resolver_lock: ctx
      .tls_port_locks
      .get(&port)
      .cloned()
      .unwrap_or_else(|| Arc::new(RwLock::new(Vec::new()))),
    tls_alpn_01_resolver_lock: ctx
      .acme_tls_alpn_01_resolver_locks
      .get(&port)
      .cloned()
      .unwrap_or_else(|| Arc::new(RwLock::new(Vec::new()))),
    http_01_resolver_lock: ctx.acme_http_01_resolvers.clone(),
    dns_provider,
    sni_hostname,
    port,
  };

  ctx.acme_on_demand_configs.push(config);
  ctx.automatic_tls_used_sni_hostnames.insert((port, None));

  Ok(())
}

/// Builds an eager (startup-time) ACME configuration.
///
/// Eager ACME requests and maintains certificates proactively at startup,
/// before any client traffic is received. This is typically used for known
/// hostnames and static configurations.
#[allow(clippy::too_many_arguments)]
fn build_eager_acme(
  ctx: &mut TlsBuildContext,
  server: &ferron_common::config::ServerConfiguration,
  port: u16,
  sni_hostname: String,
  challenge_type: ChallengeType,
  dns_provider: Option<Arc<dyn ferron_common::dns::DnsProvider + Send + Sync>>,
  crypto_provider: Arc<CryptoProvider>,
  memory_acme_account_cache_data: Arc<RwLock<HashMap<String, Vec<u8>>>>,
) -> anyhow::Result<()> {
  let certified_key_lock = Arc::new(RwLock::new(None));
  let tls_alpn_01_data_lock = Arc::new(RwLock::new(None));
  let http_01_data_lock = Arc::new(RwLock::new(None));

  let rustls_client_config =
    super::acme::build_rustls_client_config(server, crypto_provider).map_err(|e| anyhow::anyhow!(e))?;
  let (account_cache_path, certificate_cache_path) = super::acme::resolve_cache_paths(server, port, &sni_hostname)?;

  let acme_config = AcmeConfig {
    rustls_client_config,
    domains: vec![sni_hostname.clone()],
    challenge_type: challenge_type.clone(),
    contact: get_entry!("auto_tls_contact", server)
      .and_then(|e| e.values.first())
      .and_then(|v| v.as_str())
      .map(|c| vec![format!("mailto:{c}")])
      .unwrap_or_default(),
    directory: super::acme::resolve_acme_directory(server),
    eab_key: super::acme::parse_eab(server)?,
    profile: get_entry!("auto_tls_profile", server)
      .and_then(|e| e.values.first())
      .and_then(|v| v.as_str())
      .map(str::to_string),
    account_cache: if let Some(account_cache_path) = account_cache_path {
      AcmeCache::File(account_cache_path)
    } else {
      AcmeCache::Memory(memory_acme_account_cache_data)
    },
    certificate_cache: if let Some(certificate_cache_path) = certificate_cache_path {
      AcmeCache::File(certificate_cache_path)
    } else {
      AcmeCache::Memory(Default::default())
    },
    certified_key_lock: certified_key_lock.clone(),
    tls_alpn_01_data_lock: tls_alpn_01_data_lock.clone(),
    http_01_data_lock: http_01_data_lock.clone(),
    dns_provider,
    renewal_info: None,
    account: None,
  };

  ctx.acme_configs.push(acme_config);

  // Wire challenge resolvers
  match challenge_type {
    ChallengeType::Http01 => {
      ctx.acme_http_01_resolvers.blocking_write().push(http_01_data_lock);
    }
    ChallengeType::TlsAlpn01 => {
      let resolver = ctx.acme_tls_alpn_01_resolvers.entry(port).or_insert_with(|| {
        let list = Arc::new(RwLock::new(Vec::new()));
        ctx.acme_tls_alpn_01_resolver_locks.insert(port, list.clone());
        TlsAlpn01Resolver::with_resolvers(list)
      });

      resolver.load_resolver(tls_alpn_01_data_lock);
    }
    _ => {}
  }

  // Install SNI resolver
  let acme_resolver = Arc::new(crate::acme::AcmeResolver::new(certified_key_lock));
  let sni_resolver = ensure_tls_port_resolver(ctx, port);
  sni_resolver.load_host_resolver(&sni_hostname, acme_resolver);

  ctx.automatic_tls_used_sni_hostnames.insert((port, Some(sni_hostname)));

  Ok(())
}
