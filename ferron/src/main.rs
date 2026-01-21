mod acme;
mod config;
mod handler;
mod listener_handler_communication;
mod listeners;
mod request_handler;
mod runtime;
mod tls_setup;
mod util;

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use arc_swap::ArcSwap;
use async_channel::{Receiver, Sender};
use clap::{Arg, ArgAction, ArgMatches, Command};
use ferron_common::logging::{ErrorLogger, LogMessage};
use ferron_common::{get_entry, get_value, get_values};
use ferron_load_modules::{obtain_module_loaders, obtain_observability_backend_loaders};
use human_panic::{setup_panic, Metadata};
use mimalloc::MiMalloc;
use rustls::client::WebPkiServerVerifier;
use rustls::crypto::aws_lc_rs::cipher_suite::*;
use rustls::crypto::aws_lc_rs::default_provider;
use rustls::crypto::aws_lc_rs::kx_group::*;
use rustls::server::{ResolvesServerCert, WebPkiClientVerifier};
use rustls::version::{TLS12, TLS13};
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use rustls_native_certs::load_native_certs;
use rustls_platform_verifier::BuilderVerifierExt;
use shadow_rs::shadow;
use tokio_util::sync::CancellationToken;

use crate::acme::{
  add_domain_to_cache, check_certificate_validity_or_install_cached, convert_on_demand_config, get_cached_domains,
  provision_certificate, ACME_TLS_ALPN_NAME,
};
use crate::config::adapters::ConfigurationAdapter;
use crate::config::processing::{
  load_modules, merge_duplicates, premerge_configuration, remove_and_add_global_configuration,
};
use crate::config::ServerConfigurations;
use crate::handler::{create_http_handler, ReloadableHandlerData};
use crate::listener_handler_communication::ConnectionData;
use crate::listeners::{create_quic_listener, create_tcp_listener};
use crate::tls_setup::{
  handle_automatic_tls, handle_manual_tls, handle_nonencrypted_ports, manual_tls_entry, read_default_port,
  resolve_sni_hostname, should_skip_server, TlsBuildContext,
};
use crate::util::{load_certs, match_hostname, NoServerVerifier};

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
#[allow(clippy::type_complexity)]
static HANDLERS: LazyLock<Arc<Mutex<Vec<(CancellationToken, Sender<()>)>>>> =
  LazyLock::new(|| Arc::new(Mutex::new(Vec::new())));
static SERVER_CONFIG_ARCSWAP: OnceLock<Arc<ArcSwap<ReloadableHandlerData>>> = OnceLock::new();
static URING_ENABLED: LazyLock<Arc<Mutex<Option<bool>>>> = LazyLock::new(|| Arc::new(Mutex::new(None)));
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
  let mut old_runtime: Option<tokio::runtime::Runtime> = None;

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

      let default_http_port = read_default_port(global_configuration.as_deref(), false);
      let default_https_port = read_default_port(global_configuration.as_deref(), true);

      let mut tls_build_ctx = TlsBuildContext::default();
      let memory_acme_account_cache_data: Arc<tokio::sync::RwLock<HashMap<String, Vec<u8>>>> = Default::default();

      // Iterate server configurations (TLS configuration)
      for server in &server_configurations.inner {
        if should_skip_server(server) {
          continue;
        }

        let sni_hostname = resolve_sni_hostname(&server.filters);
        let https_port = server.filters.port.or(default_https_port);

        handle_nonencrypted_ports(&mut tls_build_ctx, server, default_http_port);

        if let Some(https_port) = https_port {
          let manual_tls_entry_option = manual_tls_entry(server);
          if get_entry!("auto_tls", server).is_some() || manual_tls_entry_option.is_none() {
            if let Some(error_log_message) = handle_automatic_tls(
              &mut tls_build_ctx,
              server,
              https_port,
              sni_hostname.clone(),
              crypto_provider.clone(),
              memory_acme_account_cache_data.clone(),
            )? {
              for logging_tx in global_configuration
                .as_ref()
                .map_or(&vec![], |c| &c.observability.log_channels)
              {
                logging_tx.send_blocking(error_log_message.clone()).unwrap_or_default();
              }
            } else {
              continue;
            }
          }
          if let Some((cert, key)) = manual_tls_entry(server) {
            handle_manual_tls(
              &mut tls_build_ctx,
              &crypto_provider,
              https_port,
              sni_hostname,
              cert,
              key,
            )?;
          }
        }
      }

      // If HTTP/1.1 isn't enabled, don't listen to non-encrypted ports
      if !protocols.contains(&"h1") {
        tls_build_ctx.nonencrypted_ports.clear();
      }

      for tls_port in tls_build_ctx.tls_ports.keys() {
        if tls_build_ctx.nonencrypted_ports.contains(tls_port) {
          tls_build_ctx.nonencrypted_ports.remove(tls_port);
        }
      }

      // Create TLS server configurations
      let mut quic_tls_configs = HashMap::new();
      let mut tls_configs = HashMap::new();
      let mut acme_tls_alpn_01_configs = HashMap::new();
      let certified_keys_to_preload = Arc::new(tls_build_ctx.certified_keys_to_preload);
      for (tls_port, sni_resolver) in tls_build_ctx.tls_ports.into_iter() {
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
          // TLS configuration used for QUIC listener
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
      for (tls_port, sni_resolver) in tls_build_ctx.acme_tls_alpn_01_resolvers.into_iter() {
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
      for (tcp_port, encrypted) in tls_build_ctx
        .nonencrypted_ports
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
        .and_then(|v| v.as_bool());
      let mut uring_enabled_locked = URING_ENABLED
        .lock()
        .map_err(|_| anyhow::anyhow!("Can't access the enabled `io_uring` option"))?;
      let shutdown_handlers = enable_uring != *uring_enabled_locked;
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
        if !contains {
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

      let (io_uring_disabled_tx, io_uring_disabled_rx) = async_channel::unbounded();
      if let Some(global_logger) = &global_logger {
        let global_logger = global_logger.clone();
        secondary_runtime_ref.spawn(async move {
          while let Ok(err) = io_uring_disabled_rx.recv().await {
            if let Some(err) = err {
              global_logger
                .send(LogMessage::new(
                  format!("Can't configure io_uring: {err}. Ferron may run with io_uring disabled."),
                  true,
                ))
                .await
                .unwrap_or_default();
              break;
            }
          }

          io_uring_disabled_rx.close();
        });
      } else {
        io_uring_disabled_rx.close();
      }

      let mut acme_configs = tls_build_ctx.acme_configs;
      let mut acme_configs = secondary_runtime_ref.block_on(async move {
        for acme_config in &mut acme_configs {
          // Install the certificates from the cache if they're valid
          check_certificate_validity_or_install_cached(acme_config, None)
            .await
            .unwrap_or_default();
        }
        acme_configs
      });

      let inner_handler_data = ReloadableHandlerData {
        configurations: server_configurations,
        tls_configs: Arc::new(tls_configs),
        http3_enabled: !quic_listened_socket_addresses.is_empty(),
        acme_tls_alpn_01_configs: Arc::new(acme_tls_alpn_01_configs),
        acme_http_01_resolvers: tls_build_ctx.acme_http_01_resolvers,
        enable_proxy_protocol,
      };
      let reloadable_handler_data = if let Some(data) = SERVER_CONFIG_ARCSWAP.get().cloned() {
        data.swap(Arc::new(inner_handler_data));
        data
      } else {
        let reloadable_handler_data = Arc::new(ArcSwap::from_pointee(inner_handler_data));
        let _ = SERVER_CONFIG_ARCSWAP.set(reloadable_handler_data.clone());
        reloadable_handler_data
      };

      let mut start_new_handlers = true;
      if let Ok(mut handlers_locked) = HANDLERS.lock() {
        while let Some((cancel_token, graceful_shutdown)) = handlers_locked.pop() {
          if shutdown_handlers {
            cancel_token.cancel();
          } else {
            start_new_handlers = false;
            let _ = graceful_shutdown.send_blocking(());
          }
        }
      }

      // Shut down secondary runtime
      if let Some(secondary_runtime) = old_runtime_ref.take() {
        drop(secondary_runtime);
      }

      let mut acme_on_demand_configs = tls_build_ctx.acme_on_demand_configs;
      let acme_on_demand_rx = tls_build_ctx.acme_on_demand_rx;
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

      if !acme_configs.is_empty() || !acme_on_demand_configs.is_empty() {
        // Spawn a task to handle ACME certificate provisioning, one certificate at time

        let global_configuration_clone = global_configuration.clone();
        secondary_runtime_ref.spawn(async move {
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

          let error_logger = ErrorLogger::new_multiple(
            global_configuration_clone
              .as_ref()
              .map_or(vec![], |c| c.observability.log_channels.clone()),
          );
          loop {
            for acme_config in &mut *acme_configs_mutex.lock().await {
              if let Err(acme_error) = provision_certificate(acme_config, &error_logger).await {
                error_logger
                  .log(&format!("Error while obtaining a TLS certificate: {acme_error}"))
                  .await
              }
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
          }
        });
      }

      // Spawn request handler threads
      if start_new_handlers {
        let mut handler_shutdown_channels = HANDLERS.lock().expect("Can't access the handler threads");
        for _ in 0..available_parallelism {
          handler_shutdown_channels.push(create_http_handler(
            reloadable_handler_data.clone(),
            listener_handler_rx.clone(),
            enable_uring,
            io_uring_disabled_tx.clone(),
          )?);
        }
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
            io_uring_disabled_tx.clone(),
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
              global_logger.clone(),
              first_startup,
            )?,
          );
        }
      }

      // Drop QUIC listener mutex guard
      drop(quic_listeners);

      let shutdown_result = handle_shutdown_signals(secondary_runtime_ref);

      Ok::<_, Box<dyn Error + Send + Sync>>(shutdown_result)
    };

    match execute_rest() {
      Ok(to_restart) => {
        if to_restart {
          old_runtime = Some(secondary_runtime);
          first_startup = false;
          println!("Reloading the server configuration...");
        } else {
          if let Ok(mut handlers_locked) = HANDLERS.lock() {
            while let Some((cancel_token, _)) = handlers_locked.pop() {
              cancel_token.cancel();
            }
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

    drop(module_loaders);
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
