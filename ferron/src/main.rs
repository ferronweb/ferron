mod acme;
mod config;
mod handler;
mod listener_handler_communication;
mod listeners;
mod request_handler;
mod runtime;
mod setup;
mod util;

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use arc_swap::ArcSwap;
use async_channel::{Receiver, Sender};
use clap::{Parser, ValueEnum};
use ferron_common::logging::{ErrorLogger, LogMessage};
use ferron_common::{get_entry, get_value};
use ferron_load_modules::{obtain_module_loaders, obtain_observability_backend_loaders};
use human_panic::{setup_panic, Metadata};
use mimalloc::MiMalloc;
use rustls::server::{ResolvesServerCert, WebPkiClientVerifier};
use rustls::{RootCertStore, ServerConfig};
use rustls_native_certs::load_native_certs;
use shadow_rs::shadow;
use tokio_util::sync::CancellationToken;

use crate::acme::{
  background_acme_task, check_certificate_validity_or_install_cached, convert_on_demand_config, get_cached_domains,
  ACME_TLS_ALPN_NAME,
};
use crate::config::adapters::ConfigurationAdapter;
use crate::config::processing::{
  load_modules, merge_duplicates, premerge_configuration, remove_and_add_global_configuration,
};
use crate::config::ServerConfigurations;
use crate::handler::{create_http_handler, ReloadableHandlerData};
use crate::listener_handler_communication::ConnectionData;
use crate::listeners::{create_quic_listener, create_tcp_listener};
use crate::setup::tls::{
  handle_automatic_tls, handle_manual_tls, handle_nonencrypted_ports, manual_tls_entry, read_default_port,
  resolve_sni_hostname, should_skip_server, TlsBuildContext,
};
use crate::setup::tls_single::{init_crypto_provider, set_tls_version};
use crate::util::load_certs;

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
    #[cfg(unix)]
    let configuration_reload_future = async {
      if let Ok(mut signal) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup()) {
        signal.recv().await
      } else {
        futures_util::future::pending().await
      }
    };
    #[cfg(not(unix))]
    let configuration_reload_future = async { futures_util::future::pending::<Option<()>>().await };

    let shutdown_future = async {
      if tokio::signal::ctrl_c().await.is_err() {
        futures_util::future::pending().await
      }
    };

    let continue_running = tokio::select! {
      _ = shutdown_future => {
        false
      }
      _ = configuration_reload_future => {
        true
      }
    };
    continue_running
  })
}

/// Function called before starting a server
fn before_starting_server(
  args: Args,
  configuration_adapters: HashMap<String, Box<dyn ConfigurationAdapter + Send + Sync>>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
  // Obtain the argument values
  let configuration_path: &Path = args.config.as_path();
  let configuration_adapter: &str = if let Some(config_adapter) = args.config_adapter.as_ref() {
    match config_adapter {
      ConfigAdapter::Kdl => "kdl",
      #[cfg(feature = "config-yaml-legacy")]
      ConfigAdapter::YamlLegacy => "yaml-legacy",
    }
  } else {
    determine_default_configuration_adapter(configuration_path)
  };

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
      let crypto_provider = init_crypto_provider(global_configuration.as_deref())?;

      // Install a process-wide cryptography provider. If it fails, then error it out.
      if crypto_provider.clone().install_default().is_err() && first_startup {
        Err(anyhow::anyhow!("Cannot install a process-wide cryptography provider"))?;
      }

      let crypto_provider = Arc::new(crypto_provider);

      // Build TLS configuration
      let tls_config_builder_wants_versions = ServerConfig::builder_with_provider(crypto_provider.clone());
      let tls_config_builder_wants_verifier =
        set_tls_version(tls_config_builder_wants_versions, global_configuration.as_deref())?;

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
          if get_entry!("auto_tls", server)
            .and_then(|e| e.values.first())
            .and_then(|v| v.as_bool())
            .unwrap_or(server.filters.port.is_none() && manual_tls_entry_option.is_none())
          {
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
        secondary_runtime_ref.spawn(crate::setup::metrics::background_metrics(
          metrics_channels,
          available_parallelism,
        ));
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
      let mut acme_on_demand_configs = tls_build_ctx.acme_on_demand_configs;
      let memory_acme_account_cache_data_clone = memory_acme_account_cache_data.clone();

      // Preload the cached certificates before spawning the background ACME task
      let (acme_configs, acme_on_demand_configs, existing_combinations) = secondary_runtime_ref.block_on(async move {
        let mut existing_combinations = HashSet::new();

        for acme_config in &mut acme_configs {
          // Install the certificates from the cache if they're valid
          check_certificate_validity_or_install_cached(acme_config, None)
            .await
            .unwrap_or_default();
        }

        for acme_on_demand_config in &mut acme_on_demand_configs {
          for cached_domain in get_cached_domains(acme_on_demand_config).await {
            let mut acme_config = convert_on_demand_config(
              acme_on_demand_config,
              cached_domain.clone(),
              memory_acme_account_cache_data_clone.clone(),
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

        (acme_configs, acme_on_demand_configs, existing_combinations)
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
        let acme_logger = ErrorLogger::new_multiple(
          global_configuration
            .as_ref()
            .map_or(vec![], |c| c.observability.log_channels.clone()),
        );
        secondary_runtime_ref.spawn(background_acme_task(
          acme_configs,
          acme_on_demand_configs,
          memory_acme_account_cache_data,
          acme_on_demand_rx,
          on_demand_tls_ask_endpoint,
          on_demand_tls_ask_endpoint_verify,
          acme_logger,
          crypto_provider,
          existing_combinations,
        ));
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

#[derive(Debug, Clone, PartialEq, ValueEnum)]
enum ConfigAdapter {
  Kdl,
  #[cfg(feature = "config-yaml-legacy")]
  YamlLegacy,
}

fn print_version() {
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
}

/// A fast, memory-safe web server written in Rust
#[derive(Parser, Debug)]
#[command(about, long_about = None)]
struct Args {
  /// The path to the server configuration file
  #[arg(short, long, default_value = "./ferron.kdl")]
  config: PathBuf,

  /// The configuration adapter to use
  #[arg(long, value_enum)]
  config_adapter: Option<ConfigAdapter>,

  /// Prints the used compile-time module configuration (`ferron-build.yaml` or `ferron-build-override.yaml` in the Ferron source) and exits
  #[arg(long)]
  module_config: bool,

  /// Print version and build information
  #[arg(short = 'V', long)]
  version: bool,
}

/// The main entry point of the application
fn main() {
  // Set the panic handler
  setup_panic!(Metadata::new("Ferron", env!("CARGO_PKG_VERSION"))
    .homepage("https://ferron.sh")
    .support("- Send an email message to hello@ferron.sh"));

  // Obtain the configuration adapters
  let (configuration_adapters, _all_adapters) = obtain_configuration_adapters();

  // Parse command-line arguments
  let args = Args::parse();

  if args.module_config {
    // Dump the used compile-time module configuration and exit
    println!("{}", ferron_load_modules::FERRON_BUILD_YAML);
    return;
  } else if args.version {
    print_version();
    return;
  }

  // Start the server!
  if let Err(err) = before_starting_server(args, configuration_adapters) {
    eprintln!("Error while running a server: {err}");
    std::process::exit(1);
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_supported_args() {
    let args = Args::parse_from(vec![
      "ferron",
      "--config",
      "/dev/null",
      "--config-adapter",
      "kdl",
      "--module-config",
      "--version",
    ]);
    assert!(args.module_config);
    assert!(args.version);
    assert_eq!(PathBuf::from("/dev/null"), args.config);
    assert_eq!(Some(ConfigAdapter::Kdl), args.config_adapter);
  }

  #[test]
  fn test_supported_args_short_options() {
    let args = Args::parse_from(vec![
      "ferron",
      "-c",
      "/dev/null",
      "--config-adapter",
      "kdl",
      "--module-config",
      "-V",
    ]);
    assert!(args.module_config);
    assert!(args.version);
    assert_eq!(PathBuf::from("/dev/null"), args.config);
    assert_eq!(Some(ConfigAdapter::Kdl), args.config_adapter);
  }

  #[test]
  fn test_supported_optional_args() {
    let args = Args::parse_from(vec!["ferron"]);
    assert!(!args.module_config);
    assert!(!args.version);
    assert_eq!(PathBuf::from("./ferron.kdl"), args.config);
    assert_eq!(None, args.config_adapter);
  }
}
