mod acme;
mod config;
mod handler;
mod listener_handler_communication;
mod listeners;
mod request_handler;
mod runtime;
mod tls_util;
mod util;

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;
use std::time::Duration;

use async_channel::{Receiver, Sender};
use base64::Engine;
use clap::{Arg, ArgAction, ArgMatches, Command};
use ferron_common::logging::LogMessage;
use ferron_common::{get_entry, get_value, get_values};
use ferron_load_modules::{get_dns_provider, obtain_module_loaders, obtain_observability_backend_loaders};
use human_panic::{setup_panic, Metadata};
use instant_acme::{ChallengeType, ExternalAccountKey, LetsEncrypt};
use mimalloc::MiMalloc;
use rustls::client::WebPkiServerVerifier;
use rustls::crypto::aws_lc_rs::cipher_suite::*;
use rustls::crypto::aws_lc_rs::default_provider;
use rustls::crypto::aws_lc_rs::kx_group::*;
use rustls::server::{ResolvesServerCert, WebPkiClientVerifier};
use rustls::sign::CertifiedKey;
use rustls::version::{TLS12, TLS13};
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use rustls_native_certs::load_native_certs;
use rustls_platform_verifier::BuilderVerifierExt;
use shadow_rs::shadow;
use tokio_util::sync::CancellationToken;
use xxhash_rust::xxh3::xxh3_128;

use crate::acme::{
  add_domain_to_cache, check_certificate_validity_or_install_cached, convert_on_demand_config, get_cached_domains,
  provision_certificate, AcmeCache, AcmeConfig, AcmeOnDemandConfig, AcmeResolver, TlsAlpn01Resolver,
  ACME_TLS_ALPN_NAME,
};
use crate::config::adapters::ConfigurationAdapter;
use crate::config::processing::{
  load_modules, merge_duplicates, premerge_configuration, remove_and_add_global_configuration,
};
use crate::config::ServerConfigurations;
use crate::handler::create_http_handler;
use crate::listener_handler_communication::ConnectionData;
use crate::listeners::{create_quic_listener, create_tcp_listener};
use crate::tls_util::{load_certs, load_private_key, CustomSniResolver, OneCertifiedKeyResolver};
use crate::util::{is_localhost, match_hostname, NoServerVerifier};

// Set the global allocator to use mimalloc for performance optimization
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

shadow!(build);

static LISTENER_HANDLER_CHANNEL: LazyLock<Arc<(Sender<ConnectionData>, Receiver<ConnectionData>)>> =
  LazyLock::new(|| Arc::new(async_channel::unbounded()));
#[allow(clippy::type_complexity)]
static TCP_LISTENERS: LazyLock<Arc<Mutex<HashMap<SocketAddr, CancellationToken>>>> =
  LazyLock::new(|| Arc::new(Mutex::new(HashMap::new())));
#[allow(clippy::type_complexity)]
static QUIC_LISTENERS: LazyLock<Arc<Mutex<HashMap<SocketAddr, (CancellationToken, Sender<Arc<ServerConfig>>)>>>> =
  LazyLock::new(|| Arc::new(Mutex::new(HashMap::new())));
static URING_ENABLED: LazyLock<Arc<Mutex<bool>>> = LazyLock::new(|| Arc::new(Mutex::new(true)));
static LISTENER_LOGGING_CHANNEL: LazyLock<Arc<(Sender<LogMessage>, Receiver<LogMessage>)>> =
  LazyLock::new(|| Arc::new(async_channel::unbounded()));

/// Handles shutdown signals (SIGHUP and CTRL+C) and returns whether to continue running
fn handle_shutdown_signals(runtime: &tokio::runtime::Runtime) -> bool {
  runtime.block_on(async move {
    let (continue_tx, continue_rx) = async_channel::unbounded::<bool>();
    let cancel_token = tokio_util::sync::CancellationToken::new();

    #[cfg(unix)]
    {
      let cancel_token_clone = cancel_token.clone();
      let continue_tx_clone = continue_tx.clone();
      tokio::spawn(async move {
        if let Ok(mut signal) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup()) {
          tokio::select! {
            _ = signal.recv() => {
              continue_tx_clone.send(true).await.unwrap_or_default();
            }
            _ = cancel_token_clone.cancelled() => {}
          }
        }
      });
    }

    let cancel_token_clone = cancel_token.clone();
    tokio::spawn(async move {
      tokio::select! {
        result = tokio::signal::ctrl_c() => {
          if result.is_ok() {
            continue_tx.send(false).await.unwrap_or_default();
          }
        }
        _ = cancel_token_clone.cancelled() => {}
      }
    });

    let continue_running = continue_rx.recv().await.unwrap_or(false);
    cancel_token.cancel();
    continue_running
  })
}

/// Function called before starting a server
fn before_starting_server(
  args: ArgMatches,
  configuration_adapters: HashMap<String, Box<dyn ConfigurationAdapter + Send + Sync>>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
  // Obtain the argument values
  let configuration_path: &Path = args
    .get_one::<PathBuf>("config")
    .ok_or(anyhow::anyhow!("Cannot obtain the configuration path"))?
    .as_path();
  let configuration_adapter: &str = args
    .get_one::<String>("config-adapter")
    .map_or(determine_default_configuration_adapter(configuration_path), |s| {
      s as &str
    });

  // Old handler shutdown channels and secondary runtime
  let mut old_runtime: Option<(Vec<CancellationToken>, tokio::runtime::Runtime)> = None;

  // Obtain the configuration adapter
  let configuration_adapter = configuration_adapters
    .get(configuration_adapter)
    .ok_or(anyhow::anyhow!(
      "The \"{}\" configuration adapter isn't supported",
      configuration_adapter
    ))?;

  // Determine the available parallelism
  let available_parallelism = thread::available_parallelism()?.get();

  // First startup flag
  let mut first_startup = true;

  loop {
    // Obtain the module loaders
    let mut module_loaders = obtain_module_loaders();

    // Obtain the observability backend loaders
    let mut observability_backend_loaders = obtain_observability_backend_loaders();

    // Create a secondary Tokio runtime
    let secondary_runtime = tokio::runtime::Builder::new_multi_thread()
      .worker_threads(match available_parallelism / 2 {
        0 => 1,
        non_zero => non_zero,
      })
      .thread_name("Secondary runtime")
      .enable_all()
      .build()?;

    // Load the configuration
    let configs_to_process = configuration_adapter.load_configuration(configuration_path)?;

    // Process the configurations
    let configs_to_process = merge_duplicates(configs_to_process);
    let configs_to_process = remove_and_add_global_configuration(configs_to_process);
    let configs_to_process = premerge_configuration(configs_to_process);
    let (configs_to_process, first_module_error, unused_properties) = load_modules(
      configs_to_process,
      &mut module_loaders,
      &mut observability_backend_loaders,
      &secondary_runtime,
    );

    // Finalize the configurations
    let server_configurations = Arc::new(ServerConfigurations::new(configs_to_process));

    let global_configuration = server_configurations.find_global_configuration();
    let global_configuration_clone = global_configuration.clone();

    // Reference to the secondary Tokio runtime
    let secondary_runtime_ref = &secondary_runtime;

    // Mutable reference to the old runtime
    let old_runtime_ref = &mut old_runtime;

    // Execute the rest
    let execute_rest = move || {
      if let Some(first_module_error) = first_module_error {
        // Error out if there was a module error
        Err(first_module_error)?;
      }

      // Log unused properties
      for unused_property in unused_properties {
        for logging_tx in global_configuration
          .as_ref()
          .map_or(&vec![], |c| &c.observability.log_channels)
        {
          logging_tx
            .send_blocking(LogMessage::new(
              format!("Unused configuration property detected: \"{unused_property}\""),
              true,
            ))
            .unwrap_or_default();
        }
      }

      // Configure cryptography provider for Rustls
      let mut crypto_provider = default_provider();

      // Configure cipher suites
      let cipher_suite: Vec<&config::ServerConfigurationValue> = global_configuration
        .as_deref()
        .map_or(vec![], |c| get_values!("tls_cipher_suite", c));
      if !cipher_suite.is_empty() {
        let mut cipher_suites = Vec::new();
        let cipher_suite_iter = cipher_suite.iter();
        for cipher_suite_config in cipher_suite_iter {
          if let Some(cipher_suite) = cipher_suite_config.as_str() {
            let cipher_suite_to_add = match cipher_suite {
              "TLS_AES_128_GCM_SHA256" => TLS13_AES_128_GCM_SHA256,
              "TLS_AES_256_GCM_SHA384" => TLS13_AES_256_GCM_SHA384,
              "TLS_CHACHA20_POLY1305_SHA256" => TLS13_CHACHA20_POLY1305_SHA256,
              "TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256" => TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
              "TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384" => TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
              "TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256" => TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256,
              "TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256" => TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,
              "TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384" => TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,
              "TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256" => TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256,
              _ => Err(anyhow::anyhow!(
                "The \"{}\" cipher suite is not supported",
                cipher_suite
              ))?,
            };
            cipher_suites.push(cipher_suite_to_add);
          }
        }
        crypto_provider.cipher_suites = cipher_suites;
      }

      // Configure ECDH curves
      let ecdh_curves = global_configuration
        .as_deref()
        .map_or(vec![], |c| get_values!("tls_ecdh_curve", c));
      if !ecdh_curves.is_empty() {
        let mut kx_groups = Vec::new();
        let ecdh_curves_iter = ecdh_curves.iter();
        for ecdh_curve_config in ecdh_curves_iter {
          if let Some(ecdh_curve) = ecdh_curve_config.as_str() {
            let kx_group_to_add = match ecdh_curve {
              "secp256r1" => SECP256R1,
              "secp384r1" => SECP384R1,
              "x25519" => X25519,
              "x25519mklem768" => X25519MLKEM768,
              "mklem768" => MLKEM768,
              _ => Err(anyhow::anyhow!("The \"{}\" ECDH curve is not supported", ecdh_curve))?,
            };
            kx_groups.push(kx_group_to_add);
          }
        }
        crypto_provider.kx_groups = kx_groups;
      }

      // Install a process-wide cryptography provider. If it fails, then error it out.
      if crypto_provider.clone().install_default().is_err() && first_startup {
        Err(anyhow::anyhow!("Cannot install a process-wide cryptography provider"))?;
      }

      let crypto_provider = Arc::new(crypto_provider);

      // Build TLS configuration
      let tls_config_builder_wants_versions = ServerConfig::builder_with_provider(crypto_provider.clone());

      let min_tls_version_option = global_configuration
        .as_deref()
        .and_then(|c| get_value!("tls_min_version", c))
        .and_then(|v| v.as_str());
      let max_tls_version_option = global_configuration
        .as_deref()
        .and_then(|c| get_value!("tls_max_version", c))
        .and_then(|v| v.as_str());

      let tls_config_builder_wants_verifier = if min_tls_version_option.is_none() && max_tls_version_option.is_none() {
        tls_config_builder_wants_versions.with_safe_default_protocol_versions()?
      } else {
        let tls_versions = [("TLSv1.2", &TLS12), ("TLSv1.3", &TLS13)];
        let min_tls_version_index = min_tls_version_option
          .map_or(Some(0), |v| tls_versions.iter().position(|p| p.0 == v))
          .ok_or(anyhow::anyhow!("Invalid minimum TLS version"))?;
        let max_tls_version_index = max_tls_version_option
          .map_or(Some(tls_versions.len() - 1), |v| {
            tls_versions.iter().position(|p| p.0 == v)
          })
          .ok_or(anyhow::anyhow!("Invalid maximum TLS version"))?;
        if max_tls_version_index < min_tls_version_index {
          Err(anyhow::anyhow!("Maximum TLS version is older than minimum TLS version"))?
        }
        tls_config_builder_wants_versions.with_protocol_versions(
          &tls_versions[min_tls_version_index..=max_tls_version_index]
            .iter()
            .map(|p| p.1)
            .collect::<Vec<_>>(),
        )?
      };

      let tls_config_builder_wants_server_cert = if let Some(client_cert_path) = global_configuration
        .as_deref()
        .and_then(|c| get_value!("tls_client_certificate", c))
        .and_then(|v| v.as_str())
      {
        let mut roots = RootCertStore::empty();
        let client_certificate_cas = load_certs(client_cert_path)?;
        for cert in client_certificate_cas {
          roots.add(cert)?;
        }
        tls_config_builder_wants_verifier
          .with_client_cert_verifier(WebPkiClientVerifier::builder(Arc::new(roots)).build()?)
      } else if global_configuration
        .as_deref()
        .and_then(|c| get_value!("tls_client_certificate", c))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
      {
        let roots = (|| {
          let certs_result = load_native_certs();
          if !certs_result.errors.is_empty() {
            return None;
          }
          let certs = certs_result.certs;

          let mut roots = RootCertStore::empty();
          for cert in certs {
            if roots.add(cert).is_err() {
              return None;
            }
          }

          Some(roots)
        })()
        .unwrap_or(RootCertStore {
          roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        });

        tls_config_builder_wants_verifier
          .with_client_cert_verifier(WebPkiClientVerifier::builder(Arc::new(roots)).build()?)
      } else {
        tls_config_builder_wants_verifier.with_no_client_auth()
      };

      let enable_proxy_protocol = global_configuration
        .as_ref()
        .and_then(|c| get_value!("protocol_proxy", c))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
      let protocols = global_configuration
        .as_ref()
        .and_then(|c| get_entry!("protocols", c))
        .map(|e| e.values.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
        .unwrap_or(vec!["h1", "h2"]);

      if enable_proxy_protocol && protocols.contains(&"h3") {
        Err(anyhow::anyhow!("PROXY protocol isn't supported with HTTP/3"))?
      }

      let default_http_port = global_configuration
        .as_deref()
        .and_then(|c| get_entry!("default_http_port", c))
        .and_then(|e| e.values.first())
        .map_or(Some(80), |v| {
          if v.is_null() {
            None
          } else {
            Some(v.as_i128().unwrap_or(80) as u16)
          }
        });
      let default_https_port = global_configuration
        .as_deref()
        .and_then(|c| get_entry!("default_https_port", c))
        .and_then(|e| e.values.first())
        .map_or(Some(443), |v| {
          if v.is_null() {
            None
          } else {
            Some(v.as_i128().unwrap_or(443) as u16)
          }
        });

      let mut tls_ports: HashMap<u16, CustomSniResolver> = HashMap::new();
      #[allow(clippy::type_complexity)]
      let mut tls_port_locks: HashMap<u16, Arc<tokio::sync::RwLock<Vec<(String, Arc<dyn ResolvesServerCert>)>>>> =
        HashMap::new();
      let mut nonencrypted_ports = HashSet::new();
      let mut certified_keys_to_preload: HashMap<u16, Vec<Arc<CertifiedKey>>> = HashMap::new();
      let mut used_sni_hostnames = HashSet::new();
      let mut automatic_tls_used_sni_hostnames = HashSet::new();
      let mut acme_tls_alpn_01_resolvers: HashMap<u16, TlsAlpn01Resolver> = HashMap::new();
      let mut acme_tls_alpn_01_resolver_locks: HashMap<
        u16,
        Arc<tokio::sync::RwLock<Vec<crate::acme::TlsAlpn01DataLock>>>,
      > = HashMap::new();
      let acme_http_01_resolvers: Arc<tokio::sync::RwLock<Vec<crate::acme::Http01DataLock>>> =
        Arc::new(tokio::sync::RwLock::new(Vec::new()));
      let acme_default_directory = dirs::data_local_dir().and_then(|mut p| {
        p.push("ferron-acme");
        p.into_os_string().into_string().ok()
      });
      let memory_acme_account_cache_data = Arc::new(tokio::sync::RwLock::new(HashMap::new()));
      let mut acme_configs = Vec::new();
      let mut acme_on_demand_configs = Vec::new();
      let (acme_on_demand_tx, acme_on_demand_rx) = async_channel::unbounded();
      let on_demand_tls_ask_endpoint = match global_configuration
        .as_ref()
        .and_then(|c| get_value!("auto_tls_on_demand_ask", c))
        .and_then(|v| v.as_str())
        .map(|u| u.parse::<hyper::Uri>())
      {
        Some(Ok(uri)) => Some(uri),
        Some(Err(err)) => Err(anyhow::anyhow!(
          "Failed to parse automatic TLS on demand asking endpoint URI: {}",
          err
        ))?,
        None => None,
      };
      let on_demand_tls_ask_endpoint_verify = !global_configuration
        .as_ref()
        .and_then(|c| get_value!("auto_tls_on_demand_ask_no_verification", c))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

      // Iterate server configurations (TLS configuration)
      for server_configuration in &server_configurations.inner {
        if server_configuration.filters.is_global_non_host()
          || (server_configuration.filters.is_global() && server_configuration.entries.is_empty())
        {
          // Don't add listeners from an empty global configuration or non-host global configuration
          continue;
        }

        let on_demand_tls = get_value!("auto_tls_on_demand", server_configuration)
          .and_then(|v| v.as_bool())
          .unwrap_or(false);

        let https_port = server_configuration.filters.port.or(default_https_port);

        let sni_hostname = server_configuration.filters.hostname.clone().or_else(|| {
          // !!! UNTESTED, many clients don't send SNI hostname when accessing via IP address anyway
          match server_configuration.filters.ip {
            Some(IpAddr::V4(address)) => Some(address.to_string()),
            Some(IpAddr::V6(address)) => Some(format!("[{address}]")),
            _ => None,
          }
        });

        let is_sni_hostname_used = !https_port.is_none_or(|p| {
          !used_sni_hostnames.contains(&(p, sni_hostname.clone()))
            && !automatic_tls_used_sni_hostnames.contains(&(p, sni_hostname.clone()))
        });
        let is_auto_tls_sni_hostname_used =
          https_port.is_some_and(|p| automatic_tls_used_sni_hostnames.contains(&(p, sni_hostname.clone())));

        let mut automatic_tls_port = None;
        if server_configuration.filters.port.is_none() {
          if get_value!("auto_tls", server_configuration)
            .and_then(|v| v.as_bool())
            .unwrap_or(!is_localhost(
              server_configuration.filters.ip.as_ref(),
              server_configuration.filters.hostname.as_deref(),
            ))
          {
            automatic_tls_port = default_https_port;
          }
          if let Some(http_port) = default_http_port {
            nonencrypted_ports.insert(http_port);
          }
        }

        if get_value!("auto_tls", server_configuration)
          .and_then(|v| v.as_bool())
          .unwrap_or(false)
        {
          automatic_tls_port = https_port;
        } else if let Some(tls_entry) = get_entry!("tls", server_configuration) {
          if let Some(https_port) = https_port {
            if tls_entry.values.len() == 2 {
              if let Some(cert_path) = tls_entry.values[0].as_str() {
                if let Some(key_path) = tls_entry.values[1].as_str() {
                  automatic_tls_port = None;

                  if !is_sni_hostname_used {
                    let certs = match load_certs(cert_path) {
                      Ok(certs) => certs,
                      Err(err) => Err(anyhow::anyhow!(
                        "Cannot load the \"{}\" TLS certificate: {}",
                        cert_path,
                        err
                      ))?,
                    };
                    let key = match load_private_key(key_path) {
                      Ok(key) => key,
                      Err(err) => Err(anyhow::anyhow!("Cannot load the \"{}\" private key: {}", key_path, err))?,
                    };
                    let signing_key = match crypto_provider.key_provider.load_private_key(key) {
                      Ok(key) => key,
                      Err(err) => Err(anyhow::anyhow!("Cannot load the \"{}\" private key: {}", key_path, err))?,
                    };
                    let certified_key = Arc::new(CertifiedKey::new(certs, signing_key));
                    if let Some(certified_keys) = certified_keys_to_preload.get_mut(&https_port) {
                      certified_keys.push(certified_key.clone());
                    } else {
                      certified_keys_to_preload.insert(https_port, vec![certified_key.clone()]);
                    }
                    let resolver = Arc::new(OneCertifiedKeyResolver::new(certified_key));
                    if let Some(sni_resolver) = tls_ports.get_mut(&https_port) {
                      if let Some(sni_hostname) = &sni_hostname {
                        sni_resolver.load_host_resolver(sni_hostname, resolver);
                      } else {
                        sni_resolver.load_fallback_resolver(resolver);
                      }
                    } else {
                      let sni_resolver_list = Arc::new(tokio::sync::RwLock::new(Vec::new()));
                      tls_port_locks.insert(https_port, sni_resolver_list.clone());
                      let mut sni_resolver = CustomSniResolver::with_resolvers(sni_resolver_list);
                      if let Some(sni_hostname) = &sni_hostname {
                        sni_resolver.load_host_resolver(sni_hostname, resolver);
                      } else {
                        sni_resolver.load_fallback_resolver(resolver);
                      }
                      tls_ports.insert(https_port, sni_resolver);
                    }
                    used_sni_hostnames.insert((https_port, sni_hostname.clone()));
                  }
                }
              }
            }
          }
        } else if let Some(http_port) = server_configuration.filters.port.or(default_http_port) {
          nonencrypted_ports.insert(http_port);
        }
        if let Some(automatic_tls_port) = automatic_tls_port {
          if !is_auto_tls_sni_hostname_used {
            if sni_hostname.is_some() || on_demand_tls {
              let is_wildcard_domain = sni_hostname.as_ref().is_some_and(|s| s.starts_with("*."));
              let challenge_type_entry = get_entry!("auto_tls_challenge", server_configuration);
              let challenge_type_str = challenge_type_entry
                .and_then(|e| e.values.first())
                .and_then(|v| v.as_str())
                .unwrap_or("tls-alpn-01");
              let challenge_params = challenge_type_entry
                .and_then(|e| {
                  let mut props_str = HashMap::new();
                  for (prop_name, prop_value) in e.props.iter() {
                    if let Some(prop_value) = prop_value.as_str() {
                      props_str.insert(prop_name.to_string(), prop_value.to_string());
                    }
                  }
                  if props_str.is_empty() {
                    None
                  } else {
                    Some(props_str)
                  }
                })
                .unwrap_or(HashMap::new());
              if let Some(sni_hostname) = &sni_hostname {
                if sni_hostname.parse::<IpAddr>().is_ok() {
                  for logging_tx in global_configuration
                    .as_ref()
                    .map_or(&vec![], |c| &c.observability.log_channels)
                  {
                    logging_tx
                    .send_blocking(LogMessage::new(
                      format!("Ferron's automatic TLS functionality doesn't support IP address-based identifiers, skipping SNI host \"{sni_hostname}\"..."),
                      true,
                    ))
                    .unwrap_or_default();
                  }
                  continue;
                }
              }
              let challenge_type = match &*challenge_type_str.to_uppercase() {
                "HTTP-01" => {
                  if let Some(sni_hostname) = &sni_hostname {
                    if is_wildcard_domain && !on_demand_tls {
                      for logging_tx in global_configuration
                        .as_ref()
                        .map_or(&vec![], |c| &c.observability.log_channels)
                      {
                        logging_tx
                        .send_blocking(LogMessage::new(
                          format!("HTTP-01 ACME challenge doesn't support wildcard hostnames, skipping SNI host \"{sni_hostname}\"..."),
                          true,
                        ))
                        .unwrap_or_default();
                      }
                      continue;
                    }
                  }
                  ChallengeType::Http01
                }
                "TLS-ALPN-01" => {
                  if let Some(sni_hostname) = &sni_hostname {
                    if is_wildcard_domain && !on_demand_tls {
                      for logging_tx in global_configuration
                        .as_ref()
                        .map_or(&vec![], |c| &c.observability.log_channels)
                      {
                        logging_tx
                        .send_blocking(LogMessage::new(
                          format!("TLS-ALPN-01 ACME challenge doesn't support wildcard hostnames, skipping SNI host \"{sni_hostname}\"..."),
                          true,
                        ))
                        .unwrap_or_default();
                      }
                      continue;
                    }
                  }
                  ChallengeType::TlsAlpn01
                }
                "DNS-01" => ChallengeType::Dns01,
                unsupported => Err(anyhow::anyhow!("Unsupported ACME challenge type: {}", unsupported))?,
              };
              let dns_provider: Option<Arc<dyn ferron_common::dns::DnsProvider + Send + Sync>> =
                if &*challenge_type_str.to_uppercase() != "DNS-01" {
                  None
                } else {
                  let provider_name = challenge_params
                    .get("provider")
                    .ok_or(anyhow::anyhow!("No DNS provider specified"))?;
                  Some(get_dns_provider(provider_name, &challenge_params)?)
                };
              let acme_cache_path_option = get_value!("auto_tls_cache", server_configuration)
                .map_or(acme_default_directory.as_deref(), |v| {
                  if v.is_null() {
                    None
                  } else if let Some(v) = v.as_str() {
                    Some(v)
                  } else {
                    acme_default_directory.as_deref()
                  }
                })
                .map(|path| path.to_owned());
              let rustls_client_config = (if get_value!("auto_tls_no_verification", server_configuration)
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
              {
                ClientConfig::builder_with_provider(crypto_provider.clone())
                  .with_safe_default_protocol_versions()?
                  .dangerous()
                  .with_custom_certificate_verifier(Arc::new(NoServerVerifier::new()))
              } else if let Ok(client_config) = BuilderVerifierExt::with_platform_verifier(
                ClientConfig::builder_with_provider(crypto_provider.clone()).with_safe_default_protocol_versions()?,
              ) {
                client_config
              } else {
                ClientConfig::builder_with_provider(crypto_provider.clone())
                  .with_safe_default_protocol_versions()?
                  .with_webpki_verifier(
                    WebPkiServerVerifier::builder(Arc::new(rustls::RootCertStore {
                      roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
                    }))
                    .build()?,
                  )
              })
              .with_no_client_auth();
              if on_demand_tls {
                if &*challenge_type_str.to_uppercase() == "TLS-ALPN-01" {
                  // Add TLS-ALPN-01 resolver
                  let sni_resolver_list = Arc::new(tokio::sync::RwLock::new(Vec::new()));
                  acme_tls_alpn_01_resolver_locks.insert(automatic_tls_port, sni_resolver_list.clone());
                  let sni_resolver = TlsAlpn01Resolver::with_resolvers(sni_resolver_list);
                  acme_tls_alpn_01_resolvers.insert(automatic_tls_port, sni_resolver);
                }

                if let Some(sni_resolver) = tls_ports.get_mut(&automatic_tls_port) {
                  sni_resolver.load_fallback_sender(acme_on_demand_tx.clone(), automatic_tls_port);
                } else {
                  let sni_resolver_list = Arc::new(tokio::sync::RwLock::new(Vec::new()));
                  tls_port_locks.insert(automatic_tls_port, sni_resolver_list.clone());
                  let mut sni_resolver = CustomSniResolver::with_resolvers(sni_resolver_list);
                  sni_resolver.load_fallback_sender(acme_on_demand_tx.clone(), automatic_tls_port);
                  tls_ports.insert(automatic_tls_port, sni_resolver);
                }

                let acme_on_demand_config = AcmeOnDemandConfig {
                  rustls_client_config,
                  challenge_type,
                  contact: if let Some(contact) =
                    get_value!("auto_tls_contact", server_configuration).and_then(|v| v.as_str())
                  {
                    vec![format!("mailto:{}", contact.to_string())]
                  } else {
                    vec![]
                  },
                  directory: if let Some(directory) =
                    get_value!("auto_tls_directory", server_configuration).and_then(|v| v.as_str())
                  {
                    directory.to_string()
                  } else if get_value!("auto_tls_letsencrypt_production", server_configuration)
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true)
                  {
                    LetsEncrypt::Production.url().to_string()
                  } else {
                    LetsEncrypt::Staging.url().to_string()
                  },
                  eab_key: if let Some(eab_key_entry) = get_entry!("auto_tls_eab", server_configuration) {
                    if let Some(eab_key_id) = eab_key_entry.values.first().and_then(|v| v.as_str()) {
                      if let Some(eab_key_hmac) = eab_key_entry.values.get(1).and_then(|v| v.as_str()) {
                        match base64::engine::general_purpose::URL_SAFE_NO_PAD
                          .decode(eab_key_hmac.trim_end_matches('='))
                        {
                          Ok(decoded_key) => {
                            Some(Arc::new(ExternalAccountKey::new(eab_key_id.to_string(), &decoded_key)))
                          }
                          Err(err) => Err(anyhow::anyhow!("Failed to decode EAB key HMAC: {}", err))?,
                        }
                      } else {
                        None
                      }
                    } else {
                      None
                    }
                  } else {
                    None
                  },
                  profile: get_value!("auto_tls_profile", server_configuration)
                    .and_then(|v| v.as_str().map(|s| s.to_string())),
                  cache_path: if let Some(acme_cache_path) = acme_cache_path_option.as_ref() {
                    Some(PathBuf::from_str(acme_cache_path).map_err(|_| anyhow::anyhow!("Invalid ACME cache path"))?)
                  } else {
                    None
                  },
                  sni_resolver_lock: tls_port_locks
                    .get(&automatic_tls_port)
                    .cloned()
                    .unwrap_or(Arc::new(tokio::sync::RwLock::new(Vec::new()))),
                  tls_alpn_01_resolver_lock: acme_tls_alpn_01_resolver_locks
                    .get(&automatic_tls_port)
                    .cloned()
                    .unwrap_or(Arc::new(tokio::sync::RwLock::new(Vec::new()))),
                  http_01_resolver_lock: acme_http_01_resolvers.clone(),
                  dns_provider,
                  sni_hostname: sni_hostname.clone(),
                  port: automatic_tls_port,
                };
                acme_on_demand_configs.push(acme_on_demand_config);
                automatic_tls_used_sni_hostnames.insert((automatic_tls_port, sni_hostname));
              } else if let Some(sni_hostname) = sni_hostname {
                let (account_cache_path, cert_cache_path) =
                  if let Some(acme_cache_path) = acme_cache_path_option.as_deref() {
                    let mut pathbuf =
                      PathBuf::from_str(acme_cache_path).map_err(|_| anyhow::anyhow!("Invalid ACME cache path"))?;
                    let base_pathbuf = pathbuf.clone();
                    let append_hash = base64::engine::general_purpose::URL_SAFE_NO_PAD
                      .encode(xxh3_128(format!("{automatic_tls_port}-{sni_hostname}").as_bytes()).to_be_bytes());
                    pathbuf.push(append_hash);
                    (Some(base_pathbuf), Some(pathbuf))
                  } else {
                    (None, None)
                  };
                let certified_key_lock = Arc::new(tokio::sync::RwLock::new(None));
                let tls_alpn_01_data_lock = Arc::new(tokio::sync::RwLock::new(None));
                let http_01_data_lock = Arc::new(tokio::sync::RwLock::new(None));
                let acme_config = AcmeConfig {
                  rustls_client_config,
                  domains: vec![sni_hostname.clone()],
                  challenge_type,
                  contact: if let Some(contact) =
                    get_value!("auto_tls_contact", server_configuration).and_then(|v| v.as_str())
                  {
                    vec![format!("mailto:{}", contact.to_string())]
                  } else {
                    vec![]
                  },
                  directory: if let Some(directory) =
                    get_value!("auto_tls_directory", server_configuration).and_then(|v| v.as_str())
                  {
                    directory.to_string()
                  } else if get_value!("auto_tls_letsencrypt_production", server_configuration)
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true)
                  {
                    LetsEncrypt::Production.url().to_string()
                  } else {
                    LetsEncrypt::Staging.url().to_string()
                  },
                  eab_key: if let Some(eab_key_entry) = get_entry!("auto_tls_eab", server_configuration) {
                    if let Some(eab_key_id) = eab_key_entry.values.first().and_then(|v| v.as_str()) {
                      if let Some(eab_key_hmac) = eab_key_entry.values.get(1).and_then(|v| v.as_str()) {
                        Some(Arc::new(ExternalAccountKey::new(
                          eab_key_id.to_string(),
                          &base64::engine::general_purpose::URL_SAFE_NO_PAD
                            .decode(eab_key_hmac)
                            .map_err(|err| anyhow::anyhow!("Failed to decode EAB key HMAC: {}", err))?,
                        )))
                      } else {
                        None
                      }
                    } else {
                      None
                    }
                  } else {
                    None
                  },
                  profile: get_value!("auto_tls_profile", server_configuration)
                    .and_then(|v| v.as_str().map(|s| s.to_string())),
                  account_cache: if let Some(account_cache_path) = account_cache_path {
                    AcmeCache::File(account_cache_path)
                  } else {
                    AcmeCache::Memory(memory_acme_account_cache_data.clone())
                  },
                  certificate_cache: if let Some(cert_cache_path) = cert_cache_path {
                    AcmeCache::File(cert_cache_path)
                  } else {
                    AcmeCache::Memory(Arc::new(tokio::sync::RwLock::new(HashMap::new())))
                  },
                  certified_key_lock: certified_key_lock.clone(),
                  tls_alpn_01_data_lock: tls_alpn_01_data_lock.clone(),
                  http_01_data_lock: http_01_data_lock.clone(),
                  dns_provider,
                  renewal_info: None,
                  account: None,
                };
                let acme_resolver = Arc::new(AcmeResolver::new(certified_key_lock));
                acme_configs.push(acme_config);
                match &*challenge_type_str.to_uppercase() {
                  "HTTP-01" => {
                    acme_http_01_resolvers.blocking_write().push(http_01_data_lock);
                  }
                  "TLS-ALPN-01" => {
                    if let Some(sni_resolver) = acme_tls_alpn_01_resolvers.get_mut(&automatic_tls_port) {
                      sni_resolver.load_resolver(tls_alpn_01_data_lock);
                    } else {
                      let sni_resolver_list = Arc::new(tokio::sync::RwLock::new(Vec::new()));
                      acme_tls_alpn_01_resolver_locks.insert(automatic_tls_port, sni_resolver_list.clone());
                      let sni_resolver = TlsAlpn01Resolver::with_resolvers(sni_resolver_list);
                      sni_resolver.load_resolver(tls_alpn_01_data_lock);
                      acme_tls_alpn_01_resolvers.insert(automatic_tls_port, sni_resolver);
                    }
                  }
                  _ => (),
                }
                if let Some(sni_resolver) = tls_ports.get_mut(&automatic_tls_port) {
                  sni_resolver.load_host_resolver(&sni_hostname, acme_resolver);
                } else {
                  let sni_resolver_list = Arc::new(tokio::sync::RwLock::new(Vec::new()));
                  tls_port_locks.insert(automatic_tls_port, sni_resolver_list.clone());
                  let mut sni_resolver = CustomSniResolver::with_resolvers(sni_resolver_list);
                  sni_resolver.load_host_resolver(&sni_hostname, acme_resolver);
                  tls_ports.insert(automatic_tls_port, sni_resolver);
                }
                automatic_tls_used_sni_hostnames.insert((automatic_tls_port, Some(sni_hostname)));
              }
            } else if !server_configuration.filters.is_global() && !server_configuration.filters.is_global_non_host() {
              for logging_tx in global_configuration
                .as_ref()
                .map_or(&vec![], |c| &c.observability.log_channels)
              {
                logging_tx
                  .send_blocking(LogMessage::new(
                    "Skipping automatic TLS for a host without a SNI hostname...".to_string(),
                    true,
                  ))
                  .unwrap_or_default();
              }
            }
          }
        }
      }

      // Shut down request handler threads and secondary runtime
      if let Some((handler_shutdown_channels, secondary_runtime)) = old_runtime_ref.take() {
        for shutdown in handler_shutdown_channels {
          shutdown.cancel();
        }
        drop(secondary_runtime);
      }

      if !acme_configs.is_empty() || !acme_on_demand_configs.is_empty() {
        // Spawn a task to handle ACME certificate provisioning, one certificate at time

        let global_configuration_clone = global_configuration.clone();
        secondary_runtime_ref.spawn(async move {
          for acme_config in &mut acme_configs {
            // Install the certificates from the cache if they're valid
            check_certificate_validity_or_install_cached(acme_config, None)
              .await
              .unwrap_or_default();
          }

          let mut existing_combinations = HashSet::new();
          for acme_on_demand_config in &mut acme_on_demand_configs {
            for cached_domain in get_cached_domains(acme_on_demand_config).await {
              let mut acme_config = convert_on_demand_config(
                acme_on_demand_config,
                cached_domain.clone(),
                memory_acme_account_cache_data.clone(),
              )
              .await;

              existing_combinations.insert((cached_domain, acme_on_demand_config.port));

              // Install the certificates from the cache if they're valid
              check_certificate_validity_or_install_cached(&mut acme_config, None)
                .await
                .unwrap_or_default();

              acme_configs.push(acme_config);
            }
          }

          // Wrap ACME configurations in a mutex
          let acme_configs_mutex = Arc::new(tokio::sync::Mutex::new(acme_configs));

          let prevent_file_race_conditions_mutex = Arc::new(tokio::sync::Mutex::new(()));

          if !acme_on_demand_configs.is_empty() {
            // On-demand TLS
            let acme_configs_mutex = acme_configs_mutex.clone();
            let acme_on_demand_configs = Arc::new(acme_on_demand_configs);
            let global_configuration_clone = global_configuration_clone.clone();
            tokio::spawn(async move {
              let mut existing_combinations = existing_combinations;
              while let Ok(received_data) = acme_on_demand_rx.recv().await {
                let on_demand_tls_ask_endpoint = on_demand_tls_ask_endpoint.clone();
                if let Some(on_demand_tls_ask_endpoint) = on_demand_tls_ask_endpoint {
                  let mut url_parts = on_demand_tls_ask_endpoint.into_parts();
                  if let Some(path_and_query) = url_parts.path_and_query {
                    let query = path_and_query.query();
                    let query = if let Some(query) = query {
                      format!("{}&domain={}", query, urlencoding::encode(&received_data.0))
                    } else {
                      format!("domain={}", urlencoding::encode(&received_data.0))
                    };
                    url_parts.path_and_query = Some(match format!("{}?{}", path_and_query.path(), query).parse() {
                      Ok(parsed) => parsed,
                      Err(err) => {
                        for acme_logger in global_configuration_clone
                          .as_ref()
                          .map_or(&vec![], |c| &c.observability.log_channels)
                        {
                          acme_logger
                            .send(LogMessage::new(
                              format!("Error while formatting the URL for on-demand TLS request: {err}"),
                              true,
                            ))
                            .await
                            .unwrap_or_default();
                        }
                        continue;
                      }
                    });
                  } else {
                    url_parts.path_and_query = Some(
                      match format!("/?domain={}", urlencoding::encode(&received_data.0)).parse() {
                        Ok(parsed) => parsed,
                        Err(err) => {
                          for acme_logger in global_configuration_clone
                            .as_ref()
                            .map_or(&vec![], |c| &c.observability.log_channels)
                          {
                            acme_logger
                              .send(LogMessage::new(
                                format!("Error while formatting the URL for on-demand TLS request: {err}"),
                                true,
                              ))
                              .await
                              .unwrap_or_default();
                          }
                          continue;
                        }
                      },
                    );
                  }
                  let endpoint_url = match hyper::Uri::from_parts(url_parts) {
                    Ok(parsed) => parsed,
                    Err(err) => {
                      for acme_logger in global_configuration_clone
                        .as_ref()
                        .map_or(&vec![], |c| &c.observability.log_channels)
                      {
                        acme_logger
                          .send(LogMessage::new(
                            format!("Error while formatting the URL for on-demand TLS request: {err}"),
                            true,
                          ))
                          .await
                          .unwrap_or_default();
                      }
                      continue;
                    }
                  };
                  let ask_closure = async {
                    let client = hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
                      .build::<_, http_body_util::Empty<hyper::body::Bytes>>(
                      hyper_rustls::HttpsConnectorBuilder::new()
                        .with_tls_config(
                          (if !on_demand_tls_ask_endpoint_verify {
                            ClientConfig::builder_with_provider(crypto_provider.clone())
                              .with_safe_default_protocol_versions()?
                              .dangerous()
                              .with_custom_certificate_verifier(Arc::new(NoServerVerifier::new()))
                          } else if let Ok(client_config) = BuilderVerifierExt::with_platform_verifier(
                            ClientConfig::builder_with_provider(crypto_provider.clone())
                              .with_safe_default_protocol_versions()?,
                          ) {
                            client_config
                          } else {
                            ClientConfig::builder_with_provider(crypto_provider.clone())
                              .with_safe_default_protocol_versions()?
                              .with_webpki_verifier(
                                WebPkiServerVerifier::builder(Arc::new(rustls::RootCertStore {
                                  roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
                                }))
                                .build()?,
                              )
                          })
                          .with_no_client_auth(),
                        )
                        .https_or_http()
                        .enable_http1()
                        .enable_http2()
                        .build(),
                    );
                    let request = hyper::Request::builder()
                      .method(hyper::Method::GET)
                      .uri(endpoint_url)
                      .body(http_body_util::Empty::<hyper::body::Bytes>::new())?;
                    let response = client.request(request).await?;

                    Ok::<_, Box<dyn Error + Send + Sync>>(response.status().is_success())
                  };
                  match ask_closure.await {
                    Ok(true) => (),
                    Ok(false) => {
                      for acme_logger in global_configuration_clone
                        .as_ref()
                        .map_or(&vec![], |c| &c.observability.log_channels)
                      {
                        acme_logger
                          .send(LogMessage::new(
                            format!(
                              "The TLS certificate cannot be issued for \"{}\" hostname",
                              &received_data.0
                            ),
                            true,
                          ))
                          .await
                          .unwrap_or_default();
                      }
                      continue;
                    }
                    Err(err) => {
                      for acme_logger in global_configuration_clone
                        .as_ref()
                        .map_or(&vec![], |c| &c.observability.log_channels)
                      {
                        acme_logger
                          .send(LogMessage::new(
                            format!(
                              "Error while determining if the TLS certificate can be issued for \"{}\" hostname: {err}",
                              &received_data.0
                            ),
                            true,
                          ))
                          .await
                          .unwrap_or_default();
                      }
                      continue;
                    }
                  }
                }
                if existing_combinations.contains(&received_data) {
                  continue;
                } else {
                  existing_combinations.insert(received_data.clone());
                }
                let (sni_hostname, port) = received_data;
                let acme_configs_mutex = acme_configs_mutex.clone();
                let acme_on_demand_configs = acme_on_demand_configs.clone();
                let memory_acme_account_cache_data = memory_acme_account_cache_data.clone();
                let prevent_file_race_conditions_mutex = prevent_file_race_conditions_mutex.clone();
                tokio::spawn(async move {
                  for acme_on_demand_config in acme_on_demand_configs.iter() {
                    if match_hostname(acme_on_demand_config.sni_hostname.as_deref(), Some(&sni_hostname))
                      && acme_on_demand_config.port == port
                    {
                      let mutex_guard = prevent_file_race_conditions_mutex.lock().await;
                      add_domain_to_cache(acme_on_demand_config, &sni_hostname)
                        .await
                        .unwrap_or_default();
                      drop(mutex_guard);

                      acme_configs_mutex.lock().await.push(
                        convert_on_demand_config(
                          acme_on_demand_config,
                          sni_hostname.clone(),
                          memory_acme_account_cache_data,
                        )
                        .await,
                      );
                      break;
                    }
                  }
                });
              }
            });
          }

          loop {
            for acme_config in &mut *acme_configs_mutex.lock().await {
              if let Err(acme_error) = provision_certificate(acme_config).await {
                for acme_logger in global_configuration_clone
                  .as_ref()
                  .map_or(&vec![], |c| &c.observability.log_channels)
                {
                  acme_logger
                    .send(LogMessage::new(
                      format!("Error while obtaining a TLS certificate: {acme_error}"),
                      true,
                    ))
                    .await
                    .unwrap_or_default();
                }
              }
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
          }
        });
      }

      // If HTTP/1.1 isn't enabled, don't listen to non-encrypted ports
      if !protocols.contains(&"h1") {
        nonencrypted_ports.clear();
      }

      for tls_port in tls_ports.keys() {
        if nonencrypted_ports.contains(tls_port) {
          nonencrypted_ports.remove(tls_port);
        }
      }

      // Create TLS server configurations
      let mut quic_tls_configs = HashMap::new();
      let mut tls_configs = HashMap::new();
      let mut acme_tls_alpn_01_configs = HashMap::new();
      for (tls_port, sni_resolver) in tls_ports.into_iter() {
        let enable_ocsp_stapling = global_configuration
          .as_ref()
          .and_then(|c| get_value!("ocsp_stapling", c))
          .and_then(|v| v.as_bool())
          .unwrap_or(true);
        let resolver: Arc<dyn ResolvesServerCert> = if enable_ocsp_stapling {
          // The `ocsp_stapler` crate is dependent on Tokio, so we create a stapler in the Tokio runtime...
          // If this wasn't wrapped in a Tokio runtime, creation of a OCSP stapler would just cause a panic.
          let stapler =
            secondary_runtime_ref.block_on(async move { ocsp_stapler::Stapler::new(Arc::new(sni_resolver)) });
          if let Some(certified_keys_to_preload) = certified_keys_to_preload.get(&tls_port) {
            for certified_key in certified_keys_to_preload {
              stapler.preload(certified_key.clone());
            }
          }
          Arc::new(stapler)
        } else {
          Arc::new(sni_resolver)
        };
        let mut tls_config = tls_config_builder_wants_server_cert
          .clone()
          .with_cert_resolver(resolver);
        if protocols.contains(&"h3") {
          // TLS configuration used for QUIC listene
          let mut quic_tls_config = tls_config.clone();
          quic_tls_config.max_early_data_size = u32::MAX;
          quic_tls_config.alpn_protocols.insert(0, b"h3-29".to_vec());
          quic_tls_config.alpn_protocols.insert(0, b"h3".to_vec());
          quic_tls_configs.insert(tls_port, Arc::new(quic_tls_config));
        }
        if protocols.contains(&"h1") {
          tls_config.alpn_protocols.insert(0, b"http/1.0".to_vec());
          tls_config.alpn_protocols.insert(0, b"http/1.1".to_vec());
        }
        if protocols.contains(&"h2") {
          tls_config.alpn_protocols.insert(0, b"h2".to_vec());
        }
        tls_configs.insert(tls_port, Arc::new(tls_config));
      }
      for (tls_port, sni_resolver) in acme_tls_alpn_01_resolvers.into_iter() {
        let mut tls_config = tls_config_builder_wants_server_cert
          .clone()
          .with_cert_resolver(Arc::new(sni_resolver));
        tls_config.alpn_protocols = vec![ACME_TLS_ALPN_NAME.to_vec()];
        acme_tls_alpn_01_configs.insert(tls_port, Arc::new(tls_config));
      }

      // Process metrics initialization
      #[cfg(any(target_os = "linux", target_os = "android"))]
      if let Some(metrics_channels) = global_configuration
        .as_ref()
        .map(|c| &c.observability.metric_channels)
        .cloned()
      {
        secondary_runtime_ref.spawn(async move {
          use ferron_common::observability::{Metric, MetricAttributeValue, MetricType, MetricValue};

          let mut previous_instant = std::time::Instant::now();
          let mut previous_cpu_user_time = 0.0;
          let mut previous_cpu_system_time = 0.0;
          let mut previous_rss = 0;
          let mut previous_vms = 0;
          loop {
            // Sleep for 1 second
            tokio::time::sleep(Duration::from_secs(1)).await;

            if let Ok(Ok(stat)) =
              tokio::task::spawn_blocking(|| procfs::process::Process::myself().and_then(|p| p.stat())).await
            {
              let cpu_user_time = stat.utime as f64 / procfs::ticks_per_second() as f64;
              let cpu_system_time = stat.stime as f64 / procfs::ticks_per_second() as f64;
              let cpu_user_time_increase = cpu_user_time - previous_cpu_user_time;
              let cpu_system_time_increase = cpu_system_time - previous_cpu_system_time;
              previous_cpu_user_time = cpu_user_time;
              previous_cpu_system_time = cpu_system_time;

              let rss = stat.rss * procfs::page_size();
              let rss_diff = rss as i64 - previous_rss as i64;
              let vms_diff = stat.vsize as i64 - previous_vms as i64;
              previous_rss = rss;
              previous_vms = stat.vsize;

              let elapsed = previous_instant.elapsed().as_secs_f64();
              previous_instant = std::time::Instant::now();

              let cpu_user_utilization = cpu_user_time_increase / (elapsed * available_parallelism as f64);
              let cpu_system_utilization = cpu_system_time_increase / (elapsed * available_parallelism as f64);

              for metrics_sender in &metrics_channels {
                metrics_sender
                  .send(Metric::new(
                    "process.cpu.time",
                    vec![("cpu.mode", MetricAttributeValue::String("user".to_string()))],
                    MetricType::Counter,
                    MetricValue::F64(cpu_user_time_increase),
                    Some("s"),
                    Some("Total CPU seconds broken down by different states."),
                  ))
                  .await
                  .unwrap_or_default();

                metrics_sender
                  .send(Metric::new(
                    "process.cpu.time",
                    vec![("cpu.mode", MetricAttributeValue::String("system".to_string()))],
                    MetricType::Counter,
                    MetricValue::F64(cpu_system_time_increase),
                    Some("s"),
                    Some("Total CPU seconds broken down by different states."),
                  ))
                  .await
                  .unwrap_or_default();

                metrics_sender
                  .send(Metric::new(
                    "process.cpu.utilization",
                    vec![("cpu.mode", MetricAttributeValue::String("user".to_string()))],
                    MetricType::Gauge,
                    MetricValue::F64(cpu_user_utilization),
                    Some("1"),
                    Some(
                      "Difference in process.cpu.time since the last measurement, \
                       divided by the elapsed time and number of CPUs available to the process.",
                    ),
                  ))
                  .await
                  .unwrap_or_default();

                metrics_sender
                  .send(Metric::new(
                    "process.cpu.utilization",
                    vec![("cpu.mode", MetricAttributeValue::String("system".to_string()))],
                    MetricType::Gauge,
                    MetricValue::F64(cpu_system_utilization),
                    Some("1"),
                    Some(
                      "Difference in process.cpu.time since the last measurement, \
                      divided by the elapsed time and number of CPUs available to the process.",
                    ),
                  ))
                  .await
                  .unwrap_or_default();

                metrics_sender
                  .send(Metric::new(
                    "process.memory.usage",
                    vec![],
                    MetricType::UpDownCounter,
                    MetricValue::I64(rss_diff),
                    Some("By"),
                    Some("The amount of physical memory in use."),
                  ))
                  .await
                  .unwrap_or_default();

                metrics_sender
                  .send(Metric::new(
                    "process.memory.virtual",
                    vec![],
                    MetricType::UpDownCounter,
                    MetricValue::I64(vms_diff),
                    Some("By"),
                    Some("The amount of committed virtual memory."),
                  ))
                  .await
                  .unwrap_or_default();
              }
            }
          }
        });
      }

      let (listener_handler_tx, listener_handler_rx) = &**LISTENER_HANDLER_CHANNEL;
      let mut tcp_listeners = TCP_LISTENERS
        .lock()
        .map_err(|_| anyhow::anyhow!("Can't access the TCP listeners"))?;
      let mut quic_listeners = QUIC_LISTENERS
        .lock()
        .map_err(|_| anyhow::anyhow!("Can't access the QUIC listeners"))?;
      let mut listened_socket_addresses = Vec::new();
      let mut quic_listened_socket_addresses = Vec::new();
      let listen_ip_addr = global_configuration
        .as_deref()
        .and_then(|c| get_value!("listen_ip", c))
        .and_then(|v| v.as_str())
        .map_or(Ok(IpAddr::V6(Ipv6Addr::UNSPECIFIED)), |a| a.parse())
        .map_err(|_| anyhow::anyhow!("Invalid IP address to listen to"))?;
      for (tcp_port, encrypted) in nonencrypted_ports
        .iter()
        .map(|p| (*p, false))
        .chain(tls_configs.keys().map(|p| (*p, true)))
      {
        let socket_address = SocketAddr::new(listen_ip_addr, tcp_port);
        listened_socket_addresses.push((socket_address, encrypted));
      }
      for (quic_port, quic_tls_config) in quic_tls_configs.into_iter() {
        let socket_address = SocketAddr::new(listen_ip_addr, quic_port);
        quic_listened_socket_addresses.push((socket_address, quic_tls_config));
      }

      let enable_uring = global_configuration
        .as_deref()
        .and_then(|c| get_value!("io_uring", c))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
      let mut uring_enabled_locked = URING_ENABLED
        .lock()
        .map_err(|_| anyhow::anyhow!("Can't access the enabled `io_uring` option"))?;
      let mut tcp_listener_socketaddrs_to_remove = Vec::new();
      let mut quic_listener_socketaddrs_to_remove = Vec::new();
      for (key, value) in &*tcp_listeners {
        if enable_uring != *uring_enabled_locked
          || (!listened_socket_addresses.contains(&(*key, true)) && !listened_socket_addresses.contains(&(*key, false)))
        {
          // Shut down the TCP listener
          value.cancel();

          // Push the the TCP listener address to remove
          tcp_listener_socketaddrs_to_remove.push(*key);
        }
      }
      for (key, value) in &*quic_listeners {
        let mut contains = false;
        for key2 in &quic_listened_socket_addresses {
          if key2.0 == *key {
            contains = true;
            break;
          }
        }
        if enable_uring != *uring_enabled_locked || !contains {
          // Shut down the QUIC listener
          value.0.cancel();

          // Push the the QUIC listener address to remove
          quic_listener_socketaddrs_to_remove.push(*key);
        }
      }
      *uring_enabled_locked = enable_uring;
      drop(uring_enabled_locked);

      for key_to_remove in tcp_listener_socketaddrs_to_remove {
        // Remove the TCP listener
        tcp_listeners.remove(&key_to_remove);
      }

      for key_to_remove in quic_listener_socketaddrs_to_remove {
        // Remove the QUIC listener
        quic_listeners.remove(&key_to_remove);
      }

      // Get a global logger for listeners
      let (global_logging_tx, global_logging_rx) = &**LISTENER_LOGGING_CHANNEL;
      let global_logger = if global_configuration
        .as_ref()
        .is_none_or(|c| c.observability.log_channels.is_empty())
      {
        None
      } else {
        let global_configuration_clone = global_configuration.clone();
        secondary_runtime_ref.spawn(async move {
          while let Ok(log_message) = global_logging_rx.recv().await {
            for logging_tx in global_configuration_clone
              .as_ref()
              .map_or(&vec![], |c| &c.observability.log_channels)
            {
              logging_tx.send(log_message.clone()).await.unwrap_or_default();
            }
          }
        });
        Some(global_logging_tx.clone())
      };

      // Spawn request handler threads
      let mut handler_shutdown_channels = Vec::new();
      for _ in 0..available_parallelism {
        handler_shutdown_channels.push(create_http_handler(
          server_configurations.clone(),
          listener_handler_rx.clone(),
          enable_uring,
          tls_configs.clone(),
          !quic_listened_socket_addresses.is_empty(),
          acme_tls_alpn_01_configs.clone(),
          acme_http_01_resolvers.clone(),
          enable_proxy_protocol,
        )?);
      }

      // Error out, if server is configured to listen to no port
      if listened_socket_addresses.is_empty() && quic_listened_socket_addresses.is_empty() {
        Err(anyhow::anyhow!("The server is configured to listen to no port"))?
      }

      let tcp_send_buffer_size = global_configuration
        .as_deref()
        .and_then(|c| get_value!("tcp_send_buffer", c))
        .and_then(|v| v.as_i128())
        .map(|v| v as usize);
      let tcp_recv_buffer_size = global_configuration
        .as_deref()
        .and_then(|c| get_value!("tcp_recv_buffer", c))
        .and_then(|v| v.as_i128())
        .map(|v| v as usize);
      for (socket_address, encrypted) in listened_socket_addresses {
        if let std::collections::hash_map::Entry::Vacant(e) = tcp_listeners.entry(socket_address) {
          // Create a TCP listener
          e.insert(create_tcp_listener(
            socket_address,
            encrypted,
            listener_handler_tx.clone(),
            enable_uring,
            global_logger.clone(),
            first_startup,
            (tcp_send_buffer_size, tcp_recv_buffer_size),
          )?);
        }
      }

      // Drop TCP listener mutex guard
      drop(tcp_listeners);

      for (socket_address, tls_config) in quic_listened_socket_addresses {
        if let Some(quic_listener_entry) = quic_listeners.get(&socket_address) {
          // Replace the TLS configuration in the QUIC listener
          let (_, tls_quic_listener) = quic_listener_entry;
          tls_quic_listener.send_blocking(tls_config).unwrap_or_default();
        } else {
          // Create a QUIC listener
          quic_listeners.insert(
            socket_address,
            create_quic_listener(
              socket_address,
              tls_config,
              listener_handler_tx.clone(),
              enable_uring,
              global_logger.clone(),
              first_startup,
            )?,
          );
        }
      }

      // Drop QUIC listener mutex guard
      drop(quic_listeners);

      let shutdown_result = handle_shutdown_signals(secondary_runtime_ref);

      Ok::<_, Box<dyn Error + Send + Sync>>((shutdown_result, handler_shutdown_channels))
    };

    match execute_rest() {
      Ok((to_restart, handler_shutdown_channels)) => {
        if to_restart {
          old_runtime = Some((handler_shutdown_channels, secondary_runtime));
          first_startup = false;
          println!("Reloading the server configuration...");
        } else {
          for shutdown in handler_shutdown_channels {
            shutdown.cancel();
          }
          drop(secondary_runtime);
          break;
        }
      }
      Err(err) => {
        for logging_tx in global_configuration_clone
          .as_ref()
          .map_or(&vec![], |c| &c.observability.log_channels)
        {
          logging_tx
            .send_blocking(LogMessage::new(err.to_string(), true))
            .unwrap_or_default();
        }
        std::thread::sleep(Duration::from_millis(100));
        Err(err)?
      }
    }
  }
  Ok(())
}

fn obtain_configuration_adapters() -> (
  HashMap<String, Box<dyn ConfigurationAdapter + Send + Sync>>,
  Vec<&'static str>,
) {
  // Configuration adapters
  let mut configuration_adapters: HashMap<String, Box<dyn ConfigurationAdapter + Send + Sync>> = HashMap::new();
  let mut all_adapters = Vec::new();

  // Configuration adapter registration macro
  macro_rules! register_configuration_adapter {
    ($name:literal, $adapter:expr) => {
      configuration_adapters.insert($name.to_string(), Box::new($adapter));
      all_adapters.push($name);
    };
  }

  // Register configuration adapters
  register_configuration_adapter!("kdl", config::adapters::kdl::KdlConfigurationAdapter::new());
  #[cfg(feature = "config-yaml-legacy")]
  register_configuration_adapter!(
    "yaml-legacy",
    config::adapters::yaml_legacy::YamlLegacyConfigurationAdapter::new()
  );
  #[cfg(feature = "config-docker-auto")]
  register_configuration_adapter!(
    "docker-auto",
    config::adapters::docker_auto::DockerAutoConfigurationAdapter::new()
  );

  (configuration_adapters, all_adapters)
}

/// Determines the default configuration adapter
#[cfg(feature = "config-yaml-legacy")]
fn determine_default_configuration_adapter(path: &Path) -> &'static str {
  match path
    .extension()
    .and_then(|s| s.to_str())
    .map(|s| s.to_lowercase())
    .as_deref()
  {
    Some("yaml") | Some("yml") => "yaml-legacy",
    _ => "kdl",
  }
}

/// Determines the default configuration adapter
#[cfg(not(feature = "config-yaml-legacy"))]
fn determine_default_configuration_adapter(_path: &Path) -> &'static str {
  "kdl"
}

/// Parses the command-line arguments
fn parse_arguments(all_adapters: Vec<&'static str>) -> ArgMatches {
  Command::new("Ferron")
    .about("A fast, memory-safe web server written in Rust")
    .arg(
      Arg::new("config")
        .long("config")
        .short('c')
        .help("The path to the server configuration file")
        .action(ArgAction::Set)
        .default_value("./ferron.kdl")
        .value_parser(PathBuf::from_str),
    )
    .arg(
      Arg::new("config-adapter")
        .long("config-adapter")
        .help("The configuration adapter to use")
        .action(ArgAction::Set)
        .required(false)
        .value_parser(all_adapters),
    )
    .arg(
      Arg::new("module-config")
        .long("module-config")
        .help("Prints the used compile-time module configuration (`ferron-build.yaml` or `ferron-build-override.yaml` in the Ferron source) and exits")
        .action(ArgAction::SetTrue)
    )
    .arg(
      Arg::new("version")
        .long("version")
        .short('V')
        .help("Print version and build information")
        .action(ArgAction::SetTrue)
    )
    .get_matches()
}

/// The main entry point of the application
fn main() {
  // Set the panic handler
  setup_panic!(Metadata::new("Ferron", env!("CARGO_PKG_VERSION"))
    .homepage("https://ferron.sh")
    .support("- Send an email message to hello@ferron.sh"));

  // Obtain the configuration adapters
  let (configuration_adapters, all_adapters) = obtain_configuration_adapters();

  // Parse command-line arguments
  let args = parse_arguments(all_adapters);

  if args.get_flag("module-config") {
    // Dump the used compile-time module configuration and exit
    println!("{}", ferron_load_modules::FERRON_BUILD_YAML);
    return;
  } else if args.get_flag("version") {
    // Print the server version and build information
    println!("Ferron {}", build::PKG_VERSION);
    println!("  Compiled on: {}", build::BUILD_TIME);
    println!("  Git commit: {}", build::COMMIT_HASH);
    println!("  Build target: {}", build::BUILD_TARGET);
    println!("  Rust version: {}", build::RUST_VERSION);
    println!("  Build host: {}", build::BUILD_OS);
    if shadow_rs::is_debug() {
      println!("WARNING: This is a debug build. It is not recommended for production use.");
    }
    return;
  }

  // Start the server!
  match before_starting_server(args, configuration_adapters) {
    Ok(_) => (),
    Err(err) => {
      eprintln!("Error while running a server: {err}");
      std::process::exit(1);
    }
  };
}
