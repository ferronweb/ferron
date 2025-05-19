use std::error::Error;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::SystemTime;
use std::{env, thread};

use crate::ferron_common::{LogMessage, ServerModule, ServerModuleHandlers};
use crate::ferron_request_handler::request_handler;
use crate::ferron_util::env_config;
use crate::ferron_util::load_tls::{load_certs, load_private_key};
use crate::ferron_util::sni::CustomSniResolver;
use crate::ferron_util::validate_config::{prepare_config_for_validation, validate_config};
use async_channel::Sender;
use chrono::prelude::*;
use futures_util::StreamExt;
use h3_quinn::quinn;
use h3_quinn::quinn::crypto::rustls::QuicServerConfig;
use http::Response;
use http_body_util::{BodyExt, StreamBody};
use hyper::body::{Buf, Bytes, Frame, Incoming};
use hyper::service::service_fn;
use hyper::Request;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use ocsp_stapler::Stapler;
use rustls::crypto::ring::cipher_suite::*;
use rustls::crypto::ring::default_provider;
use rustls::crypto::ring::kx_group::*;
use rustls::server::{Acceptor, WebPkiClientVerifier};
use rustls::sign::CertifiedKey;
use rustls::version::{TLS12, TLS13};
use rustls::{RootCertStore, ServerConfig};
use rustls_acme::acme::ACME_TLS_ALPN_NAME;
use rustls_acme::caches::DirCache;
use rustls_acme::{is_tls_alpn_challenge, AcmeConfig, ResolvesServerCertAcme, UseChallenge};
use rustls_native_certs::load_native_certs;
use tokio::fs;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Handle;
use tokio::signal;
use tokio::sync::Mutex;
use tokio::time;
use tokio_rustls::server::TlsStream;
use tokio_rustls::LazyConfigAcceptor;
use tokio_util::sync::CancellationToken;
use yaml_rust2::Yaml;

// Enum for maybe TLS stream
#[allow(clippy::large_enum_variant)]
enum MaybeTlsStream {
  Tls(TlsStream<TcpStream>),
  Plain(TcpStream),
}

// Function to accept and handle incoming QUIC connections
#[allow(clippy::too_many_arguments)]
async fn accept_quic_connection(
  connection_attempt: quinn::Incoming,
  local_address: SocketAddr,
  config: Arc<Yaml>,
  logger: Sender<LogMessage>,
  modules: Arc<Vec<Box<dyn ServerModule + std::marker::Send + Sync>>>,
) {
  let remote_address = connection_attempt.remote_address();

  let logger_clone = logger.clone();

  tokio::task::spawn(async move {
    match connection_attempt.await {
      Ok(connection) => {
        let mut h3_conn: h3::server::Connection<h3_quinn::Connection, Bytes> =
          match h3::server::Connection::new(h3_quinn::Connection::new(connection)).await {
            Ok(h3_conn) => h3_conn,
            Err(err) => {
              logger_clone
                .send(LogMessage::new(
                  format!("Error serving HTTP/3 connection: {}", err),
                  true,
                ))
                .await
                .unwrap_or_default();
              return;
            }
          };

        loop {
          match h3_conn.accept().await {
            Ok(Some(resolver)) => {
              let config = config.clone();
              let remote_address = remote_address;

              let logger_clone = logger_clone.clone();
              let modules = modules.clone();
              tokio::spawn(async move {
                let handlers_vec = modules
                  .iter()
                  .map(|module| module.get_handlers(Handle::current()));

                let (request, stream) = match resolver.resolve_request().await {
                  Ok(resolved) => resolved,
                  Err(err) => {
                    logger_clone
                      .send(LogMessage::new(
                        format!("Error serving HTTP/3 connection: {}", err),
                        true,
                      ))
                      .await
                      .unwrap_or_default();
                    return;
                  }
                };
                let (mut send, receive) = stream.split();
                let request_body_stream = futures_util::stream::unfold(
                  (receive, false),
                  async move |(mut receive, mut is_body_finished)| loop {
                    if !is_body_finished {
                      match receive.recv_data().await {
                        Ok(Some(mut data)) => {
                          return Some((
                            Ok(Frame::data(data.copy_to_bytes(data.remaining()))),
                            (receive, false),
                          ))
                        }
                        Ok(None) => is_body_finished = true,
                        Err(err) => {
                          return Some((
                            Err(std::io::Error::other(err.to_string())),
                            (receive, false),
                          ))
                        }
                      }
                    } else {
                      match receive.recv_trailers().await {
                        Ok(Some(trailers)) => {
                          return Some((Ok(Frame::trailers(trailers)), (receive, true)))
                        }
                        Ok(None) => {
                          return None;
                        }
                        Err(err) => {
                          return Some((
                            Err(std::io::Error::other(err.to_string())),
                            (receive, true),
                          ))
                        }
                      }
                    }
                  },
                );
                let request_body = BodyExt::boxed(StreamBody::new(request_body_stream));
                let (request_parts, _) = request.into_parts();
                let request = Request::from_parts(request_parts, request_body);
                let handlers_vec_clone = handlers_vec
                  .clone()
                  .collect::<Vec<Box<dyn ServerModuleHandlers + Send>>>();
                let mut response = match request_handler(
                  request,
                  remote_address,
                  local_address,
                  true,
                  config,
                  logger_clone.clone(),
                  handlers_vec_clone,
                  None,
                  None,
                )
                .await
                {
                  Ok(response) => response,
                  Err(err) => {
                    logger_clone
                      .send(LogMessage::new(
                        format!("Error serving HTTP/3 connection: {}", err),
                        true,
                      ))
                      .await
                      .unwrap_or_default();
                    return;
                  }
                };
                if let Ok(http_date) = httpdate::fmt_http_date(SystemTime::now()).try_into() {
                  response
                    .headers_mut()
                    .entry(http::header::DATE)
                    .or_insert(http_date);
                }
                let (response_parts, mut response_body) = response.into_parts();
                if let Err(err) = send
                  .send_response(Response::from_parts(response_parts, ()))
                  .await
                {
                  logger_clone
                    .send(LogMessage::new(
                      format!("Error serving HTTP/3 connection: {}", err),
                      true,
                    ))
                    .await
                    .unwrap_or_default();
                  return;
                }
                let mut had_trailers = false;
                while let Some(chunk) = response_body.frame().await {
                  match chunk {
                    Ok(frame) => {
                      if frame.is_data() {
                        match frame.into_data() {
                          Ok(data) => {
                            if let Err(err) = send.send_data(data).await {
                              logger_clone
                                .send(LogMessage::new(
                                  format!("Error serving HTTP/3 connection: {}", err),
                                  true,
                                ))
                                .await
                                .unwrap_or_default();
                              return;
                            }
                          }
                          Err(_) => {
                            logger_clone
                            .send(LogMessage::new(
                              "Error serving HTTP/3 connection: the frame isn't really a data frame".to_string(),
                              true,
                            ))
                            .await
                            .unwrap_or_default();
                            return;
                          }
                        }
                      } else if frame.is_trailers() {
                        match frame.into_trailers() {
                          Ok(trailers) => {
                            had_trailers = true;
                            if let Err(err) = send.send_trailers(trailers).await {
                              logger_clone
                                .send(LogMessage::new(
                                  format!("Error serving HTTP/3 connection: {}", err),
                                  true,
                                ))
                                .await
                                .unwrap_or_default();
                              return;
                            }
                          }
                          Err(_) => {
                            logger_clone
                            .send(LogMessage::new(
                              "Error serving HTTP/3 connection: the frame isn't really a trailers frame".to_string(),
                              true,
                            ))
                            .await
                            .unwrap_or_default();
                            return;
                          }
                        }
                      }
                    }
                    Err(err) => {
                      logger_clone
                        .send(LogMessage::new(
                          format!("Error serving HTTP/3 connection: {}", err),
                          true,
                        ))
                        .await
                        .unwrap_or_default();
                      return;
                    }
                  }
                }
                if !had_trailers {
                  if let Err(err) = send.finish().await {
                    logger_clone
                      .send(LogMessage::new(
                        format!("Error serving HTTP/3 connection: {}", err),
                        true,
                      ))
                      .await
                      .unwrap_or_default();
                  }
                }
              });
            }
            Ok(None) => break,
            Err(err) => {
              logger_clone
                .send(LogMessage::new(
                  format!("Error serving HTTP/3 connection: {}", err),
                  true,
                ))
                .await
                .unwrap_or_default();
              return;
            }
          }
        }
      }
      Err(err) => {
        logger_clone
          .send(LogMessage::new(
            format!("Cannot accept a connection: {}", err),
            true,
          ))
          .await
          .unwrap_or_default();
      }
    }
  });
}

// Function to accept and handle incoming connections
#[allow(clippy::too_many_arguments)]
async fn accept_connection(
  stream: TcpStream,
  remote_address: SocketAddr,
  tls_config_option: Option<(Arc<ServerConfig>, Option<Arc<ServerConfig>>)>,
  acme_http01_resolver_option: Option<Arc<ResolvesServerCertAcme>>,
  config: Arc<Yaml>,
  logger: Sender<LogMessage>,
  modules: Arc<Vec<Box<dyn ServerModule + std::marker::Send + Sync>>>,
  http3_enabled: Option<u16>,
) {
  // Disable Nagle algorithm to improve performance
  if let Err(err) = stream.set_nodelay(true) {
    logger
      .send(LogMessage::new(
        format!("Cannot disable Nagle algorithm: {}", err),
        true,
      ))
      .await
      .unwrap_or_default();
    return;
  };

  let config = config.clone();
  let local_address = match stream.local_addr() {
    Ok(local_address) => local_address,
    Err(err) => {
      logger
        .send(LogMessage::new(
          format!("Cannot obtain local address of the connection: {}", err),
          true,
        ))
        .await
        .unwrap_or_default();
      return;
    }
  };

  let logger_clone = logger.clone();

  tokio::task::spawn(async move {
    let maybe_tls_stream = if let Some((tls_config, acme_config_option)) = tls_config_option {
      let start_handshake = match LazyConfigAcceptor::new(Acceptor::default(), stream).await {
        Ok(start_handshake) => start_handshake,
        Err(err) => {
          logger
            .send(LogMessage::new(
              format!("Error during TLS handshake: {}", err),
              true,
            ))
            .await
            .unwrap_or_default();
          return;
        }
      };

      if let Some(acme_config) = acme_config_option {
        if is_tls_alpn_challenge(&start_handshake.client_hello()) {
          match start_handshake.into_stream(acme_config).await {
            Ok(_) => (),
            Err(err) => {
              logger
                .send(LogMessage::new(
                  format!("Error during TLS handshake: {}", err),
                  true,
                ))
                .await
                .unwrap_or_default();
              return;
            }
          };
          return;
        }
      }

      let tls_stream = match start_handshake.into_stream(tls_config).await {
        Ok(tls_stream) => tls_stream,
        Err(err) => {
          logger
            .send(LogMessage::new(
              format!("Error during TLS handshake: {}", err),
              true,
            ))
            .await
            .unwrap_or_default();
          return;
        }
      };

      MaybeTlsStream::Tls(tls_stream)
    } else {
      MaybeTlsStream::Plain(stream)
    };

    if let MaybeTlsStream::Tls(tls_stream) = maybe_tls_stream {
      let alpn_protocol = tls_stream.get_ref().1.alpn_protocol();
      let is_http2;

      if config["global"]["enableHTTP2"].as_bool().unwrap_or(true) {
        if alpn_protocol == Some("h2".as_bytes()) {
          is_http2 = true;
        } else {
          // Don't allow HTTP/2 if "h2" ALPN offering was't present
          is_http2 = false;
        }
      } else {
        is_http2 = false;
      }

      let io = TokioIo::new(tls_stream);
      let handlers_vec = modules
        .iter()
        .map(|module| module.get_handlers(Handle::current()));

      if is_http2 {
        let mut http2_builder = hyper::server::conn::http2::Builder::new(TokioExecutor::new());
        http2_builder.timer(TokioTimer::new());
        if let Some(initial_window_size) =
          config["global"]["http2Settings"]["initialWindowSize"].as_i64()
        {
          http2_builder.initial_stream_window_size(initial_window_size as u32);
        }
        if let Some(max_frame_size) = config["global"]["http2Settings"]["maxFrameSize"].as_i64() {
          http2_builder.max_frame_size(max_frame_size as u32);
        }
        if let Some(max_concurrent_streams) =
          config["global"]["http2Settings"]["maxConcurrentStreams"].as_i64()
        {
          http2_builder.max_concurrent_streams(max_concurrent_streams as u32);
        }
        if let Some(max_header_list_size) =
          config["global"]["http2Settings"]["maxHeaderListSize"].as_i64()
        {
          http2_builder.max_header_list_size(max_header_list_size as u32);
        }
        if let Some(enable_connect_protocol) =
          config["global"]["http2Settings"]["enableConnectProtocol"].as_bool()
        {
          if enable_connect_protocol {
            http2_builder.enable_connect_protocol();
          }
        }

        if let Err(err) = http2_builder
          .serve_connection(
            io,
            service_fn(move |request: Request<Incoming>| {
              let config = config.clone();
              let logger = logger_clone.clone();
              let handlers_vec_clone = handlers_vec
                .clone()
                .collect::<Vec<Box<dyn ServerModuleHandlers + Send>>>();
              let acme_http01_resolver_option_clone = acme_http01_resolver_option.clone();
              let (request_parts, request_body) = request.into_parts();
              let request = Request::from_parts(
                request_parts,
                request_body
                  .map_err(|e| std::io::Error::other(e.to_string()))
                  .boxed(),
              );
              request_handler(
                request,
                remote_address,
                local_address,
                true,
                config,
                logger,
                handlers_vec_clone,
                acme_http01_resolver_option_clone,
                http3_enabled,
              )
            }),
          )
          .await
        {
          logger
            .send(LogMessage::new(
              format!("Error serving HTTPS connection: {}", err),
              true,
            ))
            .await
            .unwrap_or_default();
        }
      } else {
        let mut http1_builder = hyper::server::conn::http1::Builder::new();

        // The timer is neccessary for the header timeout to work to mitigate Slowloris.
        http1_builder.timer(TokioTimer::new());

        if let Err(err) = http1_builder
          .serve_connection(
            io,
            service_fn(move |request: Request<Incoming>| {
              let config = config.clone();
              let logger = logger_clone.clone();
              let handlers_vec_clone = handlers_vec
                .clone()
                .collect::<Vec<Box<dyn ServerModuleHandlers + Send>>>();
              let acme_http01_resolver_option_clone = acme_http01_resolver_option.clone();
              let (request_parts, request_body) = request.into_parts();
              let request = Request::from_parts(
                request_parts,
                request_body
                  .map_err(|e| std::io::Error::other(e.to_string()))
                  .boxed(),
              );
              request_handler(
                request,
                remote_address,
                local_address,
                true,
                config,
                logger,
                handlers_vec_clone,
                acme_http01_resolver_option_clone,
                http3_enabled,
              )
            }),
          )
          .with_upgrades()
          .await
        {
          logger
            .send(LogMessage::new(
              format!("Error serving HTTPS connection: {}", err),
              true,
            ))
            .await
            .unwrap_or_default();
        }
      }
    } else if let MaybeTlsStream::Plain(stream) = maybe_tls_stream {
      let io = TokioIo::new(stream);
      let handlers_vec = modules
        .iter()
        .map(|module| module.get_handlers(Handle::current()));

      let mut http1_builder = hyper::server::conn::http1::Builder::new();

      // The timer is neccessary for the header timeout to work to mitigate Slowloris.
      http1_builder.timer(TokioTimer::new());

      if let Err(err) = http1_builder
        .serve_connection(
          io,
          service_fn(move |request: Request<Incoming>| {
            let config = config.clone();
            let logger = logger_clone.clone();
            let handlers_vec_clone = handlers_vec
              .clone()
              .collect::<Vec<Box<dyn ServerModuleHandlers + Send>>>();
            let acme_http01_resolver_option_clone = acme_http01_resolver_option.clone();
            let (request_parts, request_body) = request.into_parts();
            let request = Request::from_parts(
              request_parts,
              request_body
                .map_err(|e| std::io::Error::other(e.to_string()))
                .boxed(),
            );
            request_handler(
              request,
              remote_address,
              local_address,
              false,
              config,
              logger,
              handlers_vec_clone,
              acme_http01_resolver_option_clone,
              http3_enabled,
            )
          }),
        )
        .with_upgrades()
        .await
      {
        logger
          .send(LogMessage::new(
            format!("Error serving HTTP connection: {}", err),
            true,
          ))
          .await
          .unwrap_or_default();
      }
    }
  });
}

// Main server event loop
#[allow(clippy::type_complexity)]
async fn server_event_loop(
  yaml_config: Arc<Yaml>,
  logger: Sender<LogMessage>,
  modules: Vec<Box<dyn ServerModule + Send + Sync>>,
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

  for (config_to_validate, is_global, is_location, is_error_config) in prepared_config {
    match validate_config(
      config_to_validate,
      is_global,
      is_location,
      is_error_config,
      &modules_optional_builtin,
    ) {
      Ok(unused_properties) => {
        for unused_property in unused_properties {
          logger
            .send(LogMessage::new(
              format!(
                "Unused configuration property detected: \"{}\". You might load an appropriate module to use this configuration property",
                unused_property
              ),
              true,
            ))
            .await
            .unwrap_or_default();
        }
      }
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
  let acme_use_http_challenge = yaml_config["global"]["useAutomaticTLSHTTPChallenge"]
    .as_bool()
    .unwrap_or(false);
  let acme_challenge_type = if acme_use_http_challenge {
    UseChallenge::Http01
  } else {
    UseChallenge::TlsAlpn01
  };

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
  let mut acme_config = AcmeConfig::new(acme_domains).challenge_type(acme_challenge_type);
  if let Some(acme_contact_unwrapped) = acme_contact {
    acme_config = acme_config.contact_push(format!("mailto:{}", acme_contact_unwrapped));
  }
  let mut acme_config_with_cache = acme_config.cache_option(acme_cache);
  acme_config_with_cache =
    acme_config_with_cache.directory_lets_encrypt(acme_letsencrypt_production);

  let (acme_config, acme_http01_resolver) = if tls_enabled && automatic_tls_enabled {
    let mut acme_state = acme_config_with_cache.state();

    let acme_resolver = acme_state.resolver();

    // Create TLS configuration
    tls_config = if yaml_config["global"]["enableOCSPStapling"]
      .as_bool()
      .unwrap_or(true)
    {
      tls_config_builder_wants_server_cert
        .with_cert_resolver(Arc::new(Stapler::new(acme_resolver.clone())))
    } else {
      tls_config_builder_wants_server_cert.with_cert_resolver(acme_resolver.clone())
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

    if acme_use_http_challenge {
      (None, Some(acme_resolver))
    } else {
      let mut acme_config = tls_config.clone();
      acme_config.alpn_protocols.push(ACME_TLS_ALPN_NAME.to_vec());

      (Some(acme_config), None)
    }
  } else {
    // Create TLS configuration
    tls_config = if yaml_config["global"]["enableOCSPStapling"]
      .as_bool()
      .unwrap_or(true)
    {
      let ocsp_stapler_arc = Arc::new(Stapler::new(Arc::new(sni_resolver)));
      for certified_key in certified_keys.iter() {
        ocsp_stapler_arc.preload(certified_key.clone());
      }
      tls_config_builder_wants_server_cert.with_cert_resolver(ocsp_stapler_arc.clone())
    } else {
      tls_config_builder_wants_server_cert.with_cert_resolver(Arc::new(sni_resolver))
    };

    // Drop the ACME configuration
    drop(acme_config_with_cache);
    (None, None)
  };

  let quic_config = if tls_enabled
    && yaml_config["global"]["enableHTTP3"]
      .as_bool()
      .unwrap_or(false)
  {
    let mut quic_tls_config = tls_config.clone();
    quic_tls_config.max_early_data_size = u32::MAX;
    quic_tls_config.alpn_protocols = vec![b"h3".to_vec(), b"h3-29".to_vec()];
    let quic_config = quinn::ServerConfig::with_crypto(Arc::new(match QuicServerConfig::try_from(
      quic_tls_config,
    ) {
      Ok(quinn_config) => quinn_config,
      Err(err) => {
        logger
          .send(LogMessage::new(
            format!("There was a problem when starting HTTP/3 server: {}", err),
            true,
          ))
          .await
          .unwrap_or_default();
        Err(anyhow::anyhow!(format!(
          "There was a problem when starting HTTP/3 server: {}",
          err
        )))?
      }
    }));
    Some(quic_config)
  } else {
    None
  };

  // Configure ALPN protocols
  let mut alpn_protocols = vec![b"http/1.1".to_vec(), b"http/1.0".to_vec()];
  if yaml_config["global"]["enableHTTP2"]
    .as_bool()
    .unwrap_or(true)
  {
    alpn_protocols.insert(0, b"h2".to_vec());
  }
  tls_config.alpn_protocols = alpn_protocols;
  let tls_config_arc = Arc::new(tls_config);
  let acme_config_arc = acme_config.map(Arc::new);

  let mut listener = None;
  let mut listener_tls = None;
  let mut listener_quic = None;

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

    if let Some(quic_config) = quic_config {
      println!("HTTP/3 server is listening at {}", addr_tls);
      listener_quic = Some(match quinn::Endpoint::server(quic_config, addr_tls) {
        Ok(listener) => listener,
        Err(err) => {
          logger
            .send(LogMessage::new(
              format!("Cannot listen to HTTP/3 port: {}", err),
              true,
            ))
            .await
            .unwrap_or_default();
          Err(anyhow::anyhow!(format!(
            "Cannot listen to HTTP/3 port: {}",
            err
          )))?
        }
      });
    }
  }

  // Wrap the modules vector in an Arc
  let modules_arc = Arc::new(modules);

  let http3_enabled = if listener_quic.is_some() {
    Some(addr_tls.port())
  } else {
    None
  };

  // Main loop to accept incoming connections
  loop {
    let listener_borrowed = &listener;
    let listener_accept = async move {
      if let Some(listener) = listener_borrowed {
        listener.accept().await
      } else {
        futures_util::future::pending().await
      }
    };

    let listener_tls_borrowed = &listener_tls;
    let listener_tls_accept = async move {
      if let Some(listener_tls) = listener_tls_borrowed {
        listener_tls.accept().await
      } else {
        futures_util::future::pending().await
      }
    };

    let listener_quic_borrowed = &listener_quic;
    let listener_quic_accept = async move {
      if let Some(listener_quic) = listener_quic_borrowed {
        listener_quic.accept().await
      } else {
        futures_util::future::pending().await
      }
    };

    if listener_borrowed.is_none()
      && listener_tls_borrowed.is_none()
      && listener_quic_borrowed.is_none()
    {
      logger
        .send(LogMessage::new(
          String::from("No server is listening"),
          true,
        ))
        .await
        .unwrap_or_default();
      Err(anyhow::anyhow!("No server is listening"))?;
    }

    tokio::select! {
      status = listener_accept => {
        match status {
          Ok((stream, remote_address)) => {
            accept_connection(
              stream,
              remote_address,
              None,
              acme_http01_resolver.clone(),
              yaml_config.clone(),
              logger.clone(),
              modules_arc.clone(),
              None
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
      status = listener_tls_accept => {
        match status {
          Ok((stream, remote_address)) => {
            accept_connection(
              stream,
              remote_address,
              Some((tls_config_arc.clone(), acme_config_arc.clone())),
              None,
              yaml_config.clone(),
              logger.clone(),
              modules_arc.clone(),
              http3_enabled
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
      status = listener_quic_accept => {
        match status {
          Some(connection_attempt) => {
            let local_ip = SocketAddr::new(connection_attempt.local_ip().unwrap_or(IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 0))), addr_tls.port());
            accept_quic_connection(
              connection_attempt,
              local_ip,
              yaml_config.clone(),
              logger.clone(),
              modules_arc.clone()
            )
            .await;
          }
          None => {
            logger
              .send(LogMessage::new(
                "HTTP/3 connections can't be accepted anymore".to_string(),
                true,
              ))
              .await
              .unwrap_or_default();
          }
        }
      }
    };
  }
}

// Start the server
#[allow(clippy::type_complexity)]
pub fn start_server(
  yaml_config: Arc<Yaml>,
  modules: Vec<Box<dyn ServerModule + Send + Sync>>,
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
            // Safety: the environment variables are set before threads are spawned
            // The `std::env::set_var` function is safe to use in single-threaded environments
            // In Rust 2024 edition, the `std::env::set_var` function would be `unsafe`.
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

  // Log env overrides once at startup
  for msg in env_config::log_env_var_overrides() {
    logger
      .send_blocking(LogMessage::new(msg, true))
      .unwrap_or_default();
  }

  // Run the server event loop
  let result = server_runtime.block_on(async {
    let event_loop_future = server_event_loop(
      yaml_config,
      logger,
      modules,
      module_error,
      modules_optional_builtin,
      first_startup,
    );

    let (continue_tx, continue_rx) = async_channel::unbounded::<bool>();
    let cancel_token = CancellationToken::new();

    #[cfg(unix)]
    {
      let cancel_token_clone = cancel_token.clone();
      let continue_tx_clone = continue_tx.clone();
      tokio::spawn(async move {
        if let Ok(mut signal) = signal::unix::signal(signal::unix::SignalKind::hangup()) {
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
        result = signal::ctrl_c() => {
          if result.is_ok() {
            continue_tx.send(false).await.unwrap_or_default();
          }
        }
        _ = cancel_token_clone.cancelled() => {}
      }
    });

    let result = tokio::select! {
      result = event_loop_future => {
        // Sleep the Tokio runtime to ensure error logs are saved
        time::sleep(tokio::time::Duration::from_millis(100)).await;

        result.map(|_| false)
      },
      continue_running = continue_rx.recv() => Ok(continue_running?)
    };

    cancel_token.cancel();

    result
  });

  // Wait 10 seconds or until all tasks are complete
  server_runtime.shutdown_timeout(time::Duration::from_secs(10));

  result
}
