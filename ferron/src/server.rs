use std::error::Error;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::{env, thread};

use crate::ferron_request_handler::request_handler;
use crate::ferron_util::load_tls::{load_certs, load_private_key};
use crate::ferron_util::sni::CustomSniResolver;
use crate::ferron_util::validate_config::{prepare_config_for_validation, validate_config};

use async_channel::Sender;
use chrono::prelude::*;
use ferron_common::{LogMessage, ServerConfigRoot, ServerModule, ServerModuleHandlers};
use futures_util::StreamExt;
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::Request;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use libloading::Symbol;
use ocsp_stapler::Stapler;
use rustls::crypto::ring::cipher_suite::*;
use rustls::crypto::ring::default_provider;
use rustls::crypto::ring::kx_group::*;
use rustls::server::WebPkiClientVerifier;
use rustls::sign::CertifiedKey;
use rustls::version::{TLS12, TLS13};
use rustls::{RootCertStore, ServerConfig};
use rustls_native_certs::load_native_certs;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Handle;
use tokio::sync::Mutex;
use tokio::time;
use tokio::{fs, signal};
use tokio_rustls::TlsAcceptor;
use tokio_rustls_acme::caches::DirCache;
use tokio_rustls_acme::{AcmeAcceptor, AcmeConfig};
use yaml_rust2::Yaml;

// Function to accept and handle incoming connections
#[allow(clippy::too_many_arguments)]
async fn accept_connection(
  stream: TcpStream,
  remote_address: SocketAddr,
  tls_acceptor_option: Option<TlsAcceptor>,
  acme_acceptor_config_option: Option<(AcmeAcceptor, Arc<ServerConfig>)>,
  global_config_root: Arc<ServerConfigRoot>,
  host_config: Arc<Yaml>,
  logger: Sender<LogMessage>,
  modules: Arc<Vec<Box<dyn ServerModule + std::marker::Send + Sync>>>,
) {
  // Disable Nagle algorithm to improve performance
  if let Err(err) = stream.set_nodelay(true) {
    logger
      .send(LogMessage::new(
        format!("Cannot disable Nagle algorithm: {:?}", err),
        true,
      ))
      .await
      .unwrap_or_default();
    return;
  };

  let global_config_root = global_config_root.clone();
  let host_config = host_config.clone();

  let local_address = match stream.local_addr() {
    Ok(local_address) => local_address,
    Err(err) => {
      logger
        .send(LogMessage::new(
          format!("Cannot obtain local address of the connection: {:?}", err),
          true,
        ))
        .await
        .unwrap_or_default();
      return;
    }
  };

  let logger_clone = logger.clone();

  if let Some((acme_acceptor, tls_config)) = acme_acceptor_config_option {
    tokio::task::spawn(async move {
      let start_handshake = match acme_acceptor.accept(stream).await {
        Ok(Some(start_handshake)) => start_handshake,
        Ok(None) => return,
        Err(err) => {
          logger
            .send(LogMessage::new(
              format!("Error during TLS handshake: {:?}", err),
              true,
            ))
            .await
            .unwrap_or_default();
          return;
        }
      };

      let tls_stream = match start_handshake.into_stream(tls_config).await {
        Ok(tls_stream) => tls_stream,
        Err(err) => {
          logger
            .send(LogMessage::new(
              format!("Error during TLS handshake: {:?}", err),
              true,
            ))
            .await
            .unwrap_or_default();
          return;
        }
      };

      let io = TokioIo::new(tls_stream);
      let mut builder = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new());

      if let Some(enable_http2) = global_config_root.get("enableHTTP2").as_bool() {
        if !enable_http2 {
          builder = builder.http1_only();
        }
      } else {
        builder = builder.http1_only();
      }

      let mut http1_builder = &mut builder.http1();
      http1_builder = http1_builder.timer(TokioTimer::new());
      let mut http2_builder = &mut http1_builder.http2();
      http2_builder = http2_builder.timer(TokioTimer::new());
      let http2_settings = global_config_root.get("http2Settings");
      if let Some(initial_window_size) = http2_settings["initialWindowSize"].as_i64() {
        http2_builder = http2_builder.initial_stream_window_size(initial_window_size as u32);
      }
      if let Some(max_frame_size) = http2_settings["maxFrameSize"].as_i64() {
        http2_builder = http2_builder.max_frame_size(max_frame_size as u32);
      }
      if let Some(max_concurrent_streams) = http2_settings["maxConcurrentStreams"].as_i64() {
        http2_builder = http2_builder.max_concurrent_streams(max_concurrent_streams as u32);
      }
      if let Some(max_header_list_size) = http2_settings["maxHeaderListSize"].as_i64() {
        http2_builder = http2_builder.max_header_list_size(max_header_list_size as u32);
      }
      if let Some(enable_connect_protocol) = http2_settings["enableConnectProtocol"].as_bool() {
        if enable_connect_protocol {
          http2_builder = http2_builder.enable_connect_protocol();
        }
      }

      let handlers_vec = modules
        .iter()
        .map(|module| module.get_handlers(Handle::current()));

      if let Err(err) = http2_builder
        .serve_connection_with_upgrades(
          io,
          service_fn(move |request: Request<Incoming>| {
            let global_config_root = global_config_root.clone();
            let host_config = host_config.clone();
            let logger = logger_clone.clone();
            let handlers_vec_clone = handlers_vec
              .clone()
              .collect::<Vec<Box<dyn ServerModuleHandlers + Send>>>();
            let (request_parts, request_body) = request.into_parts();
            let request = Request::from_parts(request_parts, request_body.boxed());
            async move {
              request_handler(
                request,
                remote_address,
                local_address,
                true,
                global_config_root,
                host_config,
                logger,
                handlers_vec_clone,
              )
              .await
            }
          }),
        )
        .await
      {
        logger
          .send(LogMessage::new(
            format!("Error serving HTTPS connection: {:?}", err),
            true,
          ))
          .await
          .unwrap_or_default();
      }
    });
  } else if let Some(tls_acceptor) = tls_acceptor_option {
    tokio::task::spawn(async move {
      let tls_stream = match tls_acceptor.accept(stream).await {
        Ok(tls_stream) => tls_stream,
        Err(err) => {
          logger
            .send(LogMessage::new(
              format!("Error during TLS handshake: {:?}", err),
              true,
            ))
            .await
            .unwrap_or_default();
          return;
        }
      };

      let io = TokioIo::new(tls_stream);
      let mut builder = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new());

      if let Some(enable_http2) = global_config_root.get("enableHTTP2").as_bool() {
        if !enable_http2 {
          builder = builder.http1_only();
        }
      } else {
        builder = builder.http1_only();
      }

      let mut http1_builder = &mut builder.http1();
      http1_builder = http1_builder.timer(TokioTimer::new());
      let mut http2_builder = &mut http1_builder.http2();
      http2_builder = http2_builder.timer(TokioTimer::new());
      let http2_settings = global_config_root.get("http2Settings");
      if let Some(initial_window_size) = http2_settings["initialWindowSize"].as_i64() {
        http2_builder = http2_builder.initial_stream_window_size(initial_window_size as u32);
      }
      if let Some(max_frame_size) = http2_settings["maxFrameSize"].as_i64() {
        http2_builder = http2_builder.max_frame_size(max_frame_size as u32);
      }
      if let Some(max_concurrent_streams) = http2_settings["maxConcurrentStreams"].as_i64() {
        http2_builder = http2_builder.max_concurrent_streams(max_concurrent_streams as u32);
      }
      if let Some(max_header_list_size) = http2_settings["maxHeaderListSize"].as_i64() {
        http2_builder = http2_builder.max_header_list_size(max_header_list_size as u32);
      }
      if let Some(enable_connect_protocol) = http2_settings["enableConnectProtocol"].as_bool() {
        if enable_connect_protocol {
          http2_builder = http2_builder.enable_connect_protocol();
        }
      }

      let handlers_vec = modules
        .iter()
        .map(|module| module.get_handlers(Handle::current()));

      if let Err(err) = http2_builder
        .serve_connection_with_upgrades(
          io,
          service_fn(move |request: Request<Incoming>| {
            let global_config_root = global_config_root.clone();
            let host_config = host_config.clone();
            let logger = logger_clone.clone();
            let handlers_vec_clone = handlers_vec
              .clone()
              .collect::<Vec<Box<dyn ServerModuleHandlers + Send>>>();
            let (request_parts, request_body) = request.into_parts();
            let request = Request::from_parts(request_parts, request_body.boxed());
            async move {
              request_handler(
                request,
                remote_address,
                local_address,
                true,
                global_config_root,
                host_config,
                logger,
                handlers_vec_clone,
              )
              .await
            }
          }),
        )
        .await
      {
        logger
          .send(LogMessage::new(
            format!("Error serving HTTPS connection: {:?}", err),
            true,
          ))
          .await
          .unwrap_or_default();
      }
    });
  } else {
    let io = TokioIo::new(stream);
    tokio::task::spawn(async move {
      let mut builder = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new());
      if let Some(enable_http2) = global_config_root.get("enableHTTP2").as_bool() {
        if !enable_http2 {
          builder = builder.http1_only();
        }
      } else {
        builder = builder.http1_only();
      }

      let mut http1_builder = &mut builder.http1();
      http1_builder = http1_builder.timer(TokioTimer::new());
      let mut http2_builder = &mut http1_builder.http2();
      http2_builder = http2_builder.timer(TokioTimer::new());
      let http2_settings = global_config_root.get("http2Settings");
      if let Some(initial_window_size) = http2_settings["initialWindowSize"].as_i64() {
        http2_builder = http2_builder.initial_stream_window_size(initial_window_size as u32);
      }
      if let Some(max_frame_size) = http2_settings["maxFrameSize"].as_i64() {
        http2_builder = http2_builder.max_frame_size(max_frame_size as u32);
      }
      if let Some(max_concurrent_streams) = http2_settings["maxConcurrentStreams"].as_i64() {
        http2_builder = http2_builder.max_concurrent_streams(max_concurrent_streams as u32);
      }
      if let Some(max_header_list_size) = http2_settings["maxHeaderListSize"].as_i64() {
        http2_builder = http2_builder.max_header_list_size(max_header_list_size as u32);
      }
      if let Some(enable_connect_protocol) = http2_settings["enableConnectProtocol"].as_bool() {
        if enable_connect_protocol {
          http2_builder = http2_builder.enable_connect_protocol();
        }
      }

      let handlers_vec = modules
        .iter()
        .map(|module| module.get_handlers(Handle::current()));

      if let Err(err) = http2_builder
        .serve_connection_with_upgrades(
          io,
          service_fn(move |request: Request<Incoming>| {
            let global_config_root = global_config_root.clone();
            let host_config = host_config.clone();
            let logger = logger_clone.clone();
            let handlers_vec_clone = handlers_vec
              .clone()
              .collect::<Vec<Box<dyn ServerModuleHandlers + Send>>>();
            let (request_parts, request_body) = request.into_parts();
            let request = Request::from_parts(request_parts, request_body.boxed());
            async move {
              request_handler(
                request,
                remote_address,
                local_address,
                false,
                global_config_root,
                host_config,
                logger,
                handlers_vec_clone,
              )
              .await
            }
          }),
        )
        .await
      {
        logger
          .send(LogMessage::new(
            format!("Error serving HTTP connection: {:?}", err),
            true,
          ))
          .await
          .unwrap_or_default();
      }
    });
  }
}

// Main server event loop
#[allow(clippy::type_complexity)]
async fn server_event_loop(
  yaml_config: Arc<Yaml>,
  logger: Sender<LogMessage>,
  modules: Vec<Box<dyn ServerModule + Send + Sync>>,
  module_config_validation_functions: Vec<
    Symbol<'_, fn(&ServerConfigRoot, bool, bool) -> Result<(), Box<dyn Error + Send + Sync>>>,
  >,
  module_error: Option<anyhow::Error>,
  modules_optional_builtin: Vec<String>,
  first_startup: bool,
) -> Result<(), Box<dyn Error + Send + Sync>> {
  if let Some(module_error) = module_error {
    logger
      .send(LogMessage::new(module_error.to_string(), true))
      .await
      .unwrap_or_default();
    Err(module_error)?
  }

  let prepared_config = match prepare_config_for_validation(&yaml_config) {
    Ok(prepared_config) => prepared_config,
    Err(err) => {
      logger
        .send(LogMessage::new(
          format!("Server configuration validation failed: {}", err),
          true,
        ))
        .await
        .unwrap_or_default();
      Err(anyhow::anyhow!(format!(
        "Server configuration validation failed: {}",
        err
      )))?
    }
  };

  for (config_to_validate, is_global, is_location) in prepared_config {
    let config_root_to_validate = ServerConfigRoot::new(&config_to_validate);
    match validate_config(
      &config_root_to_validate,
      is_global,
      is_location,
      &modules_optional_builtin,
    ) {
      Ok(_) => (),
      Err(err) => {
        logger
          .send(LogMessage::new(
            format!("Server configuration validation failed: {}", err),
            true,
          ))
          .await
          .unwrap_or_default();
        Err(anyhow::anyhow!(format!(
          "Server configuration validation failed: {}",
          err
        )))?
      }
    };
    let module_config_validation_functions_iter = module_config_validation_functions.iter();
    for module_config_validation_function in module_config_validation_functions_iter {
      match module_config_validation_function(&config_root_to_validate, is_global, is_location) {
        Ok(_) => (),
        Err(err) => {
          logger
            .send(LogMessage::new(
              format!("Server configuration validation failed: {}", err),
              true,
            ))
            .await
            .unwrap_or_default();
          Err(anyhow::anyhow!(format!(
            "Server configuration validation failed: {}",
            err
          )))?
        }
      };
    }
  }

  let mut crypto_provider = default_provider();

  if let Some(cipher_suite) = yaml_config["global"]["cipherSuite"].as_vec() {
    let mut cipher_suites = Vec::new();
    let cipher_suite_iter = cipher_suite.iter();
    for cipher_suite_yaml in cipher_suite_iter {
      if let Some(cipher_suite) = cipher_suite_yaml.as_str() {
        let cipher_suite_to_add = match cipher_suite {
          "TLS_AES_128_GCM_SHA256" => TLS13_AES_128_GCM_SHA256,
          "TLS_AES_256_GCM_SHA384" => TLS13_AES_256_GCM_SHA384,
          "TLS_CHACHA20_POLY1305_SHA256" => TLS13_CHACHA20_POLY1305_SHA256,
          "TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256" => TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
          "TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384" => TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
          "TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256" => {
            TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256
          }
          "TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256" => TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,
          "TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384" => TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,
          "TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256" => {
            TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256
          }
          _ => {
            logger
              .send(LogMessage::new(
                format!("The \"{}\" cipher suite is not supported", cipher_suite),
                true,
              ))
              .await
              .unwrap_or_default();
            Err(anyhow::anyhow!(format!(
              "The \"{}\" cipher suite is not supported",
              cipher_suite
            )))?
          }
        };
        cipher_suites.push(cipher_suite_to_add);
      }
    }
    crypto_provider.cipher_suites = cipher_suites;
  }

  if let Some(ecdh_curves) = yaml_config["global"]["ecdhCurve"].as_vec() {
    let mut kx_groups = Vec::new();
    let ecdh_curves_iter = ecdh_curves.iter();
    for ecdh_curve_yaml in ecdh_curves_iter {
      if let Some(ecdh_curve) = ecdh_curve_yaml.as_str() {
        let kx_group_to_add = match ecdh_curve {
          "secp256r1" => SECP256R1,
          "secp384r1" => SECP384R1,
          "x25519" => X25519,
          _ => {
            logger
              .send(LogMessage::new(
                format!("The \"{}\" ECDH curve is not supported", ecdh_curve),
                true,
              ))
              .await
              .unwrap_or_default();
            Err(anyhow::anyhow!(format!(
              "The \"{}\" ECDH curve is not supported",
              ecdh_curve
            )))?
          }
        };
        kx_groups.push(kx_group_to_add);
      }
    }
    crypto_provider.kx_groups = kx_groups;
  }

  let crypto_provider_cloned = crypto_provider.clone();
  let mut sni_resolver = CustomSniResolver::new();
  let mut certified_keys = Vec::new();

  let mut automatic_tls_enabled = false;
  let mut acme_letsencrypt_production = true;

  // Read automatic TLS configuration
  if let Some(read_automatic_tls_enabled) = yaml_config["global"]["enableAutomaticTLS"].as_bool() {
    automatic_tls_enabled = read_automatic_tls_enabled;
  }

  let acme_contact = yaml_config["global"]["automaticTLSContactEmail"].as_str();
  let acme_cache = yaml_config["global"]["automaticTLSContactCacheDirectory"]
    .as_str()
    .map(|s| s.to_string())
    .map(DirCache::new);

  if let Some(read_acme_letsencrypt_production) =
    yaml_config["global"]["automaticTLSLetsEncryptProduction"].as_bool()
  {
    acme_letsencrypt_production = read_acme_letsencrypt_production;
  }

  if !automatic_tls_enabled {
    // Load public certificate and private key
    if let Some(cert_path) = yaml_config["global"]["cert"].as_str() {
      if let Some(key_path) = yaml_config["global"]["key"].as_str() {
        let certs = match load_certs(cert_path) {
          Ok(certs) => certs,
          Err(err) => {
            logger
              .send(LogMessage::new(
                format!("Cannot load the \"{}\" TLS certificate: {}", cert_path, err),
                true,
              ))
              .await
              .unwrap_or_default();
            Err(anyhow::anyhow!(format!(
              "Cannot load the \"{}\" TLS certificate: {}",
              cert_path, err
            )))?
          }
        };
        let key = match load_private_key(key_path) {
          Ok(key) => key,
          Err(err) => {
            logger
              .send(LogMessage::new(
                format!("Cannot load the \"{}\" private key: {}", cert_path, err),
                true,
              ))
              .await
              .unwrap_or_default();
            Err(anyhow::anyhow!(format!(
              "Cannot load the \"{}\" private key: {}",
              cert_path, err
            )))?
          }
        };
        let signing_key = match crypto_provider_cloned.key_provider.load_private_key(key) {
          Ok(key) => key,
          Err(err) => {
            logger
              .send(LogMessage::new(
                format!("Cannot load the \"{}\" private key: {}", cert_path, err),
                true,
              ))
              .await
              .unwrap_or_default();
            Err(anyhow::anyhow!(format!(
              "Cannot load the \"{}\" private key: {}",
              cert_path, err
            )))?
          }
        };
        let certified_key = CertifiedKey::new(certs, signing_key);
        sni_resolver.load_fallback_cert_key(Arc::new(certified_key));
      }
    }

    if let Some(sni) = yaml_config["global"]["sni"].as_hash() {
      let sni_hostnames = sni.keys();
      for sni_hostname_unknown in sni_hostnames {
        if let Some(sni_hostname) = sni_hostname_unknown.as_str() {
          if let Some(cert_path) = sni[sni_hostname_unknown]["cert"].as_str() {
            if let Some(key_path) = sni[sni_hostname_unknown]["key"].as_str() {
              let certs = match load_certs(cert_path) {
                Ok(certs) => certs,
                Err(err) => {
                  logger
                    .send(LogMessage::new(
                      format!("Cannot load the \"{}\" TLS certificate: {}", cert_path, err),
                      true,
                    ))
                    .await
                    .unwrap_or_default();
                  Err(anyhow::anyhow!(format!(
                    "Cannot load the \"{}\" TLS certificate: {}",
                    cert_path, err
                  )))?
                }
              };
              let key = match load_private_key(key_path) {
                Ok(key) => key,
                Err(err) => {
                  logger
                    .send(LogMessage::new(
                      format!("Cannot load the \"{}\" private key: {}", cert_path, err),
                      true,
                    ))
                    .await
                    .unwrap_or_default();
                  Err(anyhow::anyhow!(format!(
                    "Cannot load the \"{}\" private key: {}",
                    cert_path, err
                  )))?
                }
              };
              let signing_key = match crypto_provider_cloned.key_provider.load_private_key(key) {
                Ok(key) => key,
                Err(err) => {
                  logger
                    .send(LogMessage::new(
                      format!("Cannot load the \"{}\" private key: {}", cert_path, err),
                      true,
                    ))
                    .await
                    .unwrap_or_default();
                  Err(anyhow::anyhow!(format!(
                    "Cannot load the \"{}\" private key: {}",
                    cert_path, err
                  )))?
                }
              };
              let certified_key_arc = Arc::new(CertifiedKey::new(certs, signing_key));
              sni_resolver.load_host_cert_key(sni_hostname, certified_key_arc.clone());
              certified_keys.push(certified_key_arc);
            }
          }
        }
      }
    }
  }

  // Build TLS configuration
  let tls_config_builder_wants_versions =
    ServerConfig::builder_with_provider(Arc::new(crypto_provider_cloned));

  // Very simple minimum and maximum TLS version logic for now...
  let min_tls_version_option = yaml_config["global"]["tlsMinVersion"].as_str();
  let max_tls_version_option = yaml_config["global"]["tlsMaxVersion"].as_str();
  let tls_config_builder_wants_verifier = match min_tls_version_option {
    Some("TLSv1.3") => match max_tls_version_option {
      Some("TLSv1.2") => {
        logger
          .send(LogMessage::new(
            String::from("The maximum TLS version is older than the minimum TLS version"),
            true,
          ))
          .await
          .unwrap_or_default();
        Err(anyhow::anyhow!(String::from(
          "The maximum TLS version is older than the minimum TLS version"
        )))?
      }
      Some("TLSv1.3") | None => {
        match tls_config_builder_wants_versions.with_protocol_versions(&[&TLS13]) {
          Ok(builder) => builder,
          Err(err) => {
            logger
              .send(LogMessage::new(
                format!("Couldn't create the TLS server configuration: {}", err),
                true,
              ))
              .await
              .unwrap_or_default();
            Err(anyhow::anyhow!(format!(
              "Couldn't create the TLS server configuration: {}",
              err
            )))?
          }
        }
      }
      _ => {
        logger
          .send(LogMessage::new(
            String::from("Invalid maximum TLS version"),
            true,
          ))
          .await
          .unwrap_or_default();
        Err(anyhow::anyhow!(String::from("Invalid maximum TLS version")))?
      }
    },
    Some("TLSv1.2") | None => match max_tls_version_option {
      Some("TLSv1.2") => {
        match tls_config_builder_wants_versions.with_protocol_versions(&[&TLS12]) {
          Ok(builder) => builder,
          Err(err) => {
            logger
              .send(LogMessage::new(
                format!("Couldn't create the TLS server configuration: {}", err),
                true,
              ))
              .await
              .unwrap_or_default();
            Err(anyhow::anyhow!(format!(
              "Couldn't create the TLS server configuration: {}",
              err
            )))?
          }
        }
      }
      Some("TLSv1.3") | None => {
        match tls_config_builder_wants_versions.with_protocol_versions(&[&TLS12, &TLS13]) {
          Ok(builder) => builder,
          Err(err) => {
            logger
              .send(LogMessage::new(
                format!("Couldn't create the TLS server configuration: {}", err),
                true,
              ))
              .await
              .unwrap_or_default();
            Err(anyhow::anyhow!(format!(
              "Couldn't create the TLS server configuration: {}",
              err
            )))?
          }
        }
      }
      _ => {
        logger
          .send(LogMessage::new(
            String::from("Invalid maximum TLS version"),
            true,
          ))
          .await
          .unwrap_or_default();
        Err(anyhow::anyhow!(String::from("Invalid maximum TLS version")))?
      }
    },
    _ => {
      logger
        .send(LogMessage::new(
          String::from("Invalid minimum TLS version"),
          true,
        ))
        .await
        .unwrap_or_default();
      Err(anyhow::anyhow!(String::from("Invalid minimum TLS version")))?
    }
  };

  let tls_config_builder_wants_server_cert =
    match yaml_config["global"]["useClientCertificate"].as_bool() {
      Some(true) => {
        let mut roots = RootCertStore::empty();
        let certs_result = load_native_certs();
        if !certs_result.errors.is_empty() {
          logger
            .send(LogMessage::new(
              format!(
                "Couldn't load the native certificate store: {}",
                certs_result.errors[0]
              ),
              true,
            ))
            .await
            .unwrap_or_default();
          Err(anyhow::anyhow!(format!(
            "Couldn't load the native certificate store: {}",
            certs_result.errors[0]
          )))?
        }
        let certs = certs_result.certs;

        for cert in certs {
          match roots.add(cert) {
            Ok(_) => (),
            Err(err) => {
              logger
                .send(LogMessage::new(
                  format!(
                    "Couldn't add a certificate to the certificate store: {}",
                    err
                  ),
                  true,
                ))
                .await
                .unwrap_or_default();
              Err(anyhow::anyhow!(format!(
                "Couldn't add a certificate to the certificate store: {}",
                err
              )))?
            }
          }
        }
        tls_config_builder_wants_verifier
          .with_client_cert_verifier(WebPkiClientVerifier::builder(Arc::new(roots)).build()?)
      }
      _ => tls_config_builder_wants_verifier.with_no_client_auth(),
    };

  let mut tls_config;

  let mut addr = SocketAddr::from((IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 0)), 80));
  let mut addr_tls = SocketAddr::from((IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 0)), 443));
  let mut tls_enabled = false;
  let mut non_tls_disabled = false;

  // Install a process-wide cryptography provider. If it fails, then warn about it.
  if crypto_provider.install_default().is_err() && first_startup {
    logger
      .send(LogMessage::new(
        "Cannot install a process-wide cryptography provider".to_string(),
        true,
      ))
      .await
      .unwrap_or_default();
    Err(anyhow::anyhow!(
      "Cannot install a process-wide cryptography provider"
    ))?;
  }

  // Read port configurations from YAML
  if let Some(read_port) = yaml_config["global"]["port"].as_i64() {
    addr = SocketAddr::from((
      IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 0)),
      match read_port.try_into() {
        Ok(port) => port,
        Err(_) => {
          logger
            .send(LogMessage::new(String::from("Invalid HTTP port"), true))
            .await
            .unwrap_or_default();
          Err(anyhow::anyhow!("Invalid HTTP port"))?
        }
      },
    ));
  } else if let Some(read_port) = yaml_config["global"]["port"].as_str() {
    addr = match read_port.parse() {
      Ok(addr) => addr,
      Err(_) => {
        logger
          .send(LogMessage::new(String::from("Invalid HTTP port"), true))
          .await
          .unwrap_or_default();
        Err(anyhow::anyhow!("Invalid HTTP port"))?
      }
    };
  }

  if let Some(read_tls_enabled) = yaml_config["global"]["secure"].as_bool() {
    tls_enabled = read_tls_enabled;
    if let Some(read_non_tls_disabled) =
      yaml_config["global"]["disableNonEncryptedServer"].as_bool()
    {
      non_tls_disabled = read_non_tls_disabled;
    }
  }

  if let Some(read_port) = yaml_config["global"]["sport"].as_i64() {
    addr_tls = SocketAddr::from((
      IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 0)),
      match read_port.try_into() {
        Ok(port) => port,
        Err(_) => {
          logger
            .send(LogMessage::new(String::from("Invalid HTTPS port"), true))
            .await
            .unwrap_or_default();
          Err(anyhow::anyhow!("Invalid HTTPS port"))?
        }
      },
    ));
  } else if let Some(read_port) = yaml_config["global"]["sport"].as_str() {
    addr_tls = match read_port.parse() {
      Ok(addr) => addr,
      Err(_) => {
        logger
          .send(LogMessage::new(String::from("Invalid HTTPS port"), true))
          .await
          .unwrap_or_default();
        Err(anyhow::anyhow!("Invalid HTTPS port"))?
      }
    };
  }

  // Get domains for ACME configuration
  let mut acme_domains = Vec::new();
  if let Some(hosts_config) = yaml_config["hosts"].as_vec() {
    for host_yaml in hosts_config.iter() {
      if let Some(host) = host_yaml.as_hash() {
        if let Some(domain_yaml) = host.get(&Yaml::from_str("domain")) {
          if let Some(domain) = domain_yaml.as_str() {
            if !domain.contains("*") {
              acme_domains.push(domain);
            }
          }
        }
      }
    }
  }

  // Create ACME configuration
  let mut acme_config = AcmeConfig::new(acme_domains);
  if let Some(acme_contact_unwrapped) = acme_contact {
    acme_config = acme_config.contact_push(format!("mailto:{}", acme_contact_unwrapped));
  }
  let mut acme_config_with_cache = acme_config.cache_option(acme_cache);
  acme_config_with_cache =
    acme_config_with_cache.directory_lets_encrypt(acme_letsencrypt_production);

  let acme_tls_acceptor = if tls_enabled && automatic_tls_enabled {
    let mut acme_state = acme_config_with_cache.state();
    let acceptor = acme_state.acceptor();

    // Create TLS configuration
    tls_config = match yaml_config["global"]["enableOCSPStapling"].as_bool() {
      Some(true) => tls_config_builder_wants_server_cert
        .with_cert_resolver(Arc::new(Stapler::new(acme_state.resolver()))),
      _ => tls_config_builder_wants_server_cert.with_cert_resolver(acme_state.resolver()),
    };

    let acme_logger = logger.clone();
    tokio::spawn(async move {
      while let Some(acme_result) = acme_state.next().await {
        if let Err(acme_error) = acme_result {
          acme_logger
            .send(LogMessage::new(
              format!("Error while obtaining a TLS certificate: {}", acme_error),
              true,
            ))
            .await
            .unwrap_or_default();
        }
      }
    });

    Some(acceptor)
  } else {
    // Create TLS configuration
    tls_config = match yaml_config["global"]["enableOCSPStapling"].as_bool() {
      Some(true) => {
        let ocsp_stapler_arc = Arc::new(Stapler::new(Arc::new(sni_resolver)));
        for certified_key in certified_keys.iter() {
          ocsp_stapler_arc.preload(certified_key.clone());
        }
        tls_config_builder_wants_server_cert.with_cert_resolver(ocsp_stapler_arc.clone())
      }
      _ => tls_config_builder_wants_server_cert.with_cert_resolver(Arc::new(sni_resolver)),
    };

    // Drop the ACME configuration
    drop(acme_config_with_cache);
    None
  };

  // Configure ALPN protocols
  let mut alpn_protocols = vec![b"http/1.1".to_vec(), b"http/1.0".to_vec()];
  if let Some(enable_http2) = yaml_config["global"]["enableHTTP2"].as_bool() {
    if enable_http2 {
      alpn_protocols.insert(0, b"h2".to_vec());
    }
  }
  tls_config.alpn_protocols = alpn_protocols;
  let tls_config_arc = Arc::new(tls_config);

  let acme_tls_acceptor_and_config =
    acme_tls_acceptor.map(|acceptor| (acceptor, tls_config_arc.clone()));

  // Create TLS acceptor
  let tls_acceptor = TlsAcceptor::from(tls_config_arc.clone());

  let mut listener = None;
  let mut listener_tls = None;

  // Bind to the specified ports
  if !non_tls_disabled {
    println!("HTTP server is listening at {}", addr);
    listener = Some(match TcpListener::bind(addr).await {
      Ok(listener) => listener,
      Err(err) => {
        logger
          .send(LogMessage::new(
            format!("Cannot listen to HTTP port: {}", err),
            true,
          ))
          .await
          .unwrap_or_default();
        Err(anyhow::anyhow!(format!(
          "Cannot listen to HTTP port: {}",
          err
        )))?
      }
    });
  }

  if tls_enabled {
    println!("HTTPS server is listening at {}", addr_tls);
    listener_tls = Some(match TcpListener::bind(addr_tls).await {
      Ok(listener) => listener,
      Err(err) => {
        logger
          .send(LogMessage::new(
            format!("Cannot listen to HTTPS port: {}", err),
            true,
          ))
          .await
          .unwrap_or_default();
        Err(anyhow::anyhow!(format!(
          "Cannot listen to HTTPS port: {}",
          err
        )))?
      }
    });
  }

  // Wrap the modules vector in an Arc
  let modules_arc = Arc::new(modules);

  // Create a global configuration root
  let global_config_root = Arc::new(ServerConfigRoot::new(&yaml_config["global"]));
  let host_config = Arc::new(yaml_config["hosts"].clone());

  // Main loop to accept incoming connections
  loop {
    match &listener {
      Some(listener) => match &listener_tls {
        Some(listener_tls) => {
          tokio::select! {
              status = listener.accept() => {
                match status {
                  Ok((stream, remote_address)) => {
                    accept_connection(
                      stream,
                      remote_address,
                      None,
                      None,
                      global_config_root.clone(),
                      host_config.clone(),
                      logger.clone(),
                      modules_arc.clone(),
                    )
                    .await;
                  }
                  Err(err) => {
                    logger
                      .send(LogMessage::new(
                        format!("Cannot accept a connection: {}", err),
                        true,
                      ))
                      .await
                      .unwrap_or_default();
                  }
                }
              },
              status = listener_tls.accept() => {
                match status {
                  Ok((stream, remote_address)) => {
                    let tls_acceptor = tls_acceptor.clone();
                    accept_connection(
                      stream,
                      remote_address,
                      Some(tls_acceptor),
                      acme_tls_acceptor_and_config.clone(),
                      global_config_root.clone(),
                      host_config.clone(),
                      logger.clone(),
                      modules_arc.clone(),
                    )
                    .await;
                  }
                  Err(err) => {
                    logger
                      .send(LogMessage::new(
                        format!("Cannot accept a connection: {}", err),
                        true,
                      ))
                      .await
                      .unwrap_or_default();
                  }
                }
              }
          };
        }
        None => match listener.accept().await {
          Ok((stream, remote_address)) => {
            accept_connection(
              stream,
              remote_address,
              None,
              None,
              global_config_root.clone(),
              host_config.clone(),
              logger.clone(),
              modules_arc.clone(),
            )
            .await;
          }
          Err(err) => {
            logger
              .send(LogMessage::new(
                format!("Cannot accept a connection: {}", err),
                true,
              ))
              .await
              .unwrap_or_default();
          }
        },
      },
      None => {
        match &listener_tls {
          Some(listener_tls) => match listener_tls.accept().await {
            Ok((stream, remote_address)) => {
              let tls_acceptor = tls_acceptor.clone();
              accept_connection(
                stream,
                remote_address,
                Some(tls_acceptor),
                acme_tls_acceptor_and_config.clone(),
                global_config_root.clone(),
                host_config.clone(),
                logger.clone(),
                modules_arc.clone(),
              )
              .await;
            }
            Err(err) => {
              logger
                .send(LogMessage::new(
                  format!("Cannot accept a connection: {}", err),
                  true,
                ))
                .await
                .unwrap_or_default();
            }
          },
          None => {
            // No server is listening...
            logger
              .send(LogMessage::new(
                String::from("No server is listening"),
                true,
              ))
              .await
              .unwrap_or_default();
            Err(anyhow::anyhow!("No server is listening"))?;
          }
        }
      }
    }
  }
}

// Start the server
#[allow(clippy::type_complexity)]
pub fn start_server(
  yaml_config: Arc<Yaml>,
  modules: Vec<Box<dyn ServerModule + Send + Sync>>,
  module_config_validation_functions: Vec<
    Symbol<'_, fn(&ServerConfigRoot, bool, bool) -> Result<(), Box<dyn Error + Send + Sync>>>,
  >,
  module_error: Option<anyhow::Error>,
  modules_optional_builtin: Vec<String>,
  first_startup: bool,
) -> Result<bool, Box<dyn Error + Send + Sync>> {
  if let Some(environment_variables_hash) = yaml_config["global"]["environmentVariables"].as_hash()
  {
    let environment_variables_hash_iter = environment_variables_hash.iter();
    for (variable_name, variable_value) in environment_variables_hash_iter {
      if let Some(variable_name) = variable_name.as_str() {
        if let Some(variable_value) = variable_value.as_str() {
          if !variable_name.is_empty()
            && !variable_name.contains('\0')
            && !variable_name.contains('=')
            && !variable_value.contains('\0')
          {
            env::set_var(variable_name, variable_value);
          }
        }
      }
    }
  }

  let available_parallelism = thread::available_parallelism()?.get();

  // Create Tokio runtime for the server
  let server_runtime = tokio::runtime::Builder::new_multi_thread()
    .worker_threads(available_parallelism)
    .max_blocking_threads(1536)
    .event_interval(25)
    .thread_name("server-pool")
    .enable_all()
    .build()?;

  // Create Tokio runtime for logging
  let log_runtime = tokio::runtime::Builder::new_multi_thread()
    .worker_threads(match available_parallelism / 2 {
      0 => 1,
      non_zero => non_zero,
    })
    .max_blocking_threads(768)
    .thread_name("log-pool")
    .enable_time()
    .build()?;

  let (logger, receive_log) = async_channel::bounded::<LogMessage>(10000);

  let log_filename = yaml_config["global"]["logFilePath"]
    .as_str()
    .map(String::from);
  let error_log_filename = yaml_config["global"]["errorLogFilePath"]
    .as_str()
    .map(String::from);

  log_runtime.spawn(async move {
    let log_file = match log_filename {
      Some(log_filename) => Some(
        fs::OpenOptions::new()
          .append(true)
          .create(true)
          .open(log_filename)
          .await,
      ),
      None => None,
    };

    let error_log_file = match error_log_filename {
      Some(error_log_filename) => Some(
        fs::OpenOptions::new()
          .append(true)
          .create(true)
          .open(error_log_filename)
          .await,
      ),
      None => None,
    };

    let log_file_wrapped = match log_file {
      Some(Ok(file)) => Some(Arc::new(Mutex::new(BufWriter::with_capacity(131072, file)))),
      Some(Err(e)) => {
        eprintln!("Failed to open log file: {}", e);
        None
      }
      None => None,
    };

    let error_log_file_wrapped = match error_log_file {
      Some(Ok(file)) => Some(Arc::new(Mutex::new(BufWriter::with_capacity(131072, file)))),
      Some(Err(e)) => {
        eprintln!("Failed to open error log file: {}", e);
        None
      }
      None => None,
    };

    // The logs are written when the log message is received by the log event loop, and flushed every 100 ms, improving the server performance.
    let log_file_wrapped_cloned_for_sleep = log_file_wrapped.clone();
    let error_log_file_wrapped_cloned_for_sleep = error_log_file_wrapped.clone();
    tokio::task::spawn(async move {
      let mut interval = time::interval(time::Duration::from_millis(100));
      loop {
        interval.tick().await;
        if let Some(log_file_wrapped_cloned) = log_file_wrapped_cloned_for_sleep.clone() {
          let mut locked_file = log_file_wrapped_cloned.lock().await;
          locked_file.flush().await.unwrap_or_default();
        }
        if let Some(error_log_file_wrapped_cloned) = error_log_file_wrapped_cloned_for_sleep.clone()
        {
          let mut locked_file = error_log_file_wrapped_cloned.lock().await;
          locked_file.flush().await.unwrap_or_default();
        }
      }
    });

    // Logging loop
    while let Ok(message) = receive_log.recv().await {
      let (mut message, is_error) = message.get_message();
      let log_file_wrapped_cloned = if !is_error {
        log_file_wrapped.clone()
      } else {
        error_log_file_wrapped.clone()
      };

      if let Some(log_file_wrapped_cloned) = log_file_wrapped_cloned {
        tokio::task::spawn(async move {
          let mut locked_file = log_file_wrapped_cloned.lock().await;
          if is_error {
            let now: DateTime<Local> = Local::now();
            let formatted_time = now.format("%Y-%m-%d %H:%M:%S").to_string();
            message = format!("[{}]: {}", formatted_time, message);
          }
          message.push('\n');
          if let Err(e) = locked_file.write(message.as_bytes()).await {
            eprintln!("Failed to write to log file: {}", e);
          }
        });
      }
    }
  });

  // Run the server event loop
  server_runtime.block_on(async {
    let event_loop_future = server_event_loop(
      yaml_config,
      logger,
      modules,
      module_config_validation_functions,
      module_error,
      modules_optional_builtin,
      first_startup,
    );

    #[cfg(unix)]
    {
      match signal::unix::signal(signal::unix::SignalKind::hangup()) {
        Ok(mut signal) => {
          tokio::select! {
            result = event_loop_future => {
              // Sleep the Tokio runtime to ensure error logs are saved
              time::sleep(tokio::time::Duration::from_millis(100)).await;

              result.map(|_| false)
            },
            _ = signal.recv() => Ok(true)
          }
        }
        Err(_) => {
          let result = event_loop_future.await;

          // Sleep the Tokio runtime to ensure error logs are saved
          time::sleep(tokio::time::Duration::from_millis(100)).await;

          result.map(|_| false)
        }
      }
    }

    #[cfg(not(unix))]
    {
      let result = event_loop_future.await;

      // Sleep the Tokio runtime to ensure error logs are saved
      time::sleep(tokio::time::Duration::from_millis(100)).await;

      result.map(|_| false)
    }
  })
}
