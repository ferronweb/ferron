//! TCP listener and connection handling

use std::io;
use std::net::SocketAddr;
#[cfg(unix)]
use std::os::fd::{AsRawFd, BorrowedFd};
#[cfg(windows)]
use std::os::windows::io::{AsRawSocket, BorrowedSocket};
use std::sync::Arc;
use std::time::Instant;

use ferron_core::runtime::Runtime;
use ferron_core::{log_error, log_info, log_warn};
use ferron_observability::{CompositeEventSink, LogAttributeValue, TraceSampler};
use ferron_tls::observability::{
    emit_connections_active, emit_handshake_duration, emit_handshake_total,
};
use ferron_tls::TlsConnectionParams;
use rustls::server::Acceptor;
use tokio_util::sync::CancellationToken;

use crate::util::proxy_protocol::read_proxy_header;

use super::common::*;
use super::native_sockets::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TcpListenerOptions {
    pub address: SocketAddr,
    pub send_buffer_size: Option<usize>,
    pub recv_buffer_size: Option<usize>,
    pub backlog: Option<i32>,
    pub multipath: bool,
}

pub struct TcpListenerHandle {
    cancel_token: Arc<CancellationToken>,
}

impl TcpListenerHandle {
    pub fn new(
        options: TcpListenerOptions,
        http3_alt_svc: bool,
        config: ConfigArcSwap,
        runtime: &mut Runtime,
    ) -> Result<Self, std::io::Error> {
        let listener = build_tcp_listener(
            options.address,
            (options.send_buffer_size, options.recv_buffer_size),
            options.backlog,
            options.multipath,
        )?;

        if config.load().tls_resolver.is_some() {
            log_info!("HTTPS server listening on {}", options.address);
        } else {
            log_info!("HTTP server listening on {}", options.address);
        }

        let cancel_token = Arc::new(CancellationToken::new());

        let config_clone = config.clone();
        let cancel_token_clone = cancel_token.clone();

        runtime.spawn_primary_task(move || {
            let new_listener_result = listener.try_clone();
            let cancel_token = cancel_token_clone.clone();
            let config = config_clone.clone();
            Box::pin(async move {
                let Ok(new_listener) = new_listener_result else {
                    log_error!("Failed to clone listener");
                    return;
                };

                // On Windows, `from_std` would fail with cloned sockets due to IOCP not
                // allowing multiple completion ports for the same socket, so `from_std_poll`
                // is used instead (it uses \Device\Afd, used in zincio async runtime)
                #[cfg(not(windows))]
                let Ok(listener) = zincio::net::TcpListener::from_std(new_listener) else {
                    log_error!("Failed to convert listener to zincio");
                    return;
                };
                #[cfg(windows)]
                let Ok(listener) = zincio::net::TcpListener::from_std_poll(new_listener) else {
                    log_error!("Failed to convert listener to zincio");
                    return;
                };

                #[cfg(unix)]
                let mut handle_exhaustion_backoff = std::time::Duration::from_millis(10);
                loop {
                    let accept_result = tokio::select! {
                        res = listener.accept() => res,
                        _ = cancel_token.cancelled() => {
                            return;
                        }
                    };
                    let (socket, _) = match accept_result {
                        Ok(socket) => {
                            #[cfg(unix)]
                            {
                                handle_exhaustion_backoff = std::time::Duration::from_millis(10);
                            }
                            socket
                        }
                        Err(err) => {
                            let global_observability =
                                resolve_root_observability_sink(&config.load().observability_resolver, Some(&ferron_observability::TraceSampler::new(&config.load().trace_sampling)));
                            emit_error(
                                &global_observability,
                                format!("Failed to accept connection: {err}"),
                                vec![("error.type", LogAttributeValue::String("tcp_accept_error".into())),
                                    (
                                        "error.message",
                                        LogAttributeValue::String(err.to_string()),
                                    )],
                            );
                            emit_connection_error_metric(&global_observability, "tcp", "accept");
                            #[cfg(unix)]
                            if err.raw_os_error() == Some(24) {
                                zincio::time::sleep(handle_exhaustion_backoff).await;
                                handle_exhaustion_backoff =
                                    handle_exhaustion_backoff.saturating_mul(2);
                                if handle_exhaustion_backoff > std::time::Duration::from_secs(1) {
                                    handle_exhaustion_backoff = std::time::Duration::from_secs(1);
                                }
                            }
                            continue;
                        }
                    };
                    let _ = socket.set_nodelay(true);

                    // Set TCP buffer sizes on the accepted socket, not just the listener socket.
                    {
                        #[cfg(unix)]
                        let socket_handle = unsafe { BorrowedFd::borrow_raw(socket.as_raw_fd()) };
                        #[cfg(windows)]
                        let socket_handle = unsafe { BorrowedSocket::borrow_raw(socket.as_raw_socket()) };
                        let sock2 = socket2::SockRef::from(&socket_handle);
                        if let Some(send_buffer_size) = options.send_buffer_size {
                            sock2
                                .set_send_buffer_size(send_buffer_size)
                                .unwrap_or_default();
                        }
                        if let Some(recv_buffer_size) = options.recv_buffer_size {
                            sock2
                                .set_recv_buffer_size(recv_buffer_size)
                                .unwrap_or_default();
                        }
                    }

                    let Ok(socket) = socket.into_poll() else {
                        let global_observability =
                            resolve_root_observability_sink(&config.load().observability_resolver, Some(&ferron_observability::TraceSampler::new(&config.load().trace_sampling)));
                        emit_error(
                            &global_observability,
                            "Failed to convert socket to poll-based I/O",
                            vec![(
                                "error.type",
                                LogAttributeValue::String("tcp_socket_setup_error".into()),
                            )],
                        );
                        emit_connection_error_metric(
                            &global_observability,
                            "tcp",
                            "socket_setup",
                        );
                        continue;
                    };

                    let server_config = config.load_full();
                    let connection_cancel_token = cancel_token.clone();
                    zincio::spawn_detached(async move {
                        let _conn_guard = ConnectionCountGuard::new();

                        // Read PROXY protocol header
                        // Use root HttpConnectionOptions to determine if PROXY protocol is enabled
                        let proxy_protocol_enabled = server_config
                            .http_connection_options_resolver
                            .root_data()
                            .map(|opts| opts.proxy_protocol_enabled)
                            .unwrap_or(false);
                        let (socket, proxy_client_addr, proxy_server_addr) = if proxy_protocol_enabled {
                            // Use tokio's TcpStream to read PROXY header asynchronously
                            match read_proxy_header(socket).await {
                                Ok((stream, client_addr, server_addr)) => {
                                    // Convert back to std TcpStream for zincio
                                    (stream, client_addr, server_addr)
                                }
                                Err(e) => {
                                    let global_observability =
                                        resolve_root_observability_sink(&server_config.observability_resolver, Some(&ferron_observability::TraceSampler::new(&server_config.trace_sampling)));
                                    emit_error(
                                        &global_observability,
                                        format!("Failed to read PROXY protocol header: {e}"),
                                        vec![(
                                            "error.type",
                                            LogAttributeValue::String("tcp_proxy_protocol_error".into()),
                                        ),
                                        (
                                            "error.message",
                                            LogAttributeValue::String(e.to_string()),
                                        )],
                                    );
                                    emit_connection_error_metric(
                                        &global_observability,
                                        "tcp",
                                        "proxy_protocol",
                                    );
                                    return;
                                }
                            }
                        } else {
                            (socket, None, None)
                        };

                        // Use PROXY protocol addresses if available, otherwise get from socket
                        let (remote_addr, local_addr) = if let (Some(client), Some(server)) =
                            (proxy_client_addr, proxy_server_addr)
                        {
                            (client, server)
                        } else {
                            let Ok(remote_addr) = socket.peer_addr() else {
                                let global_observability =
                                    resolve_root_observability_sink(&server_config.observability_resolver, Some(&ferron_observability::TraceSampler::new(&server_config.trace_sampling)));
                                emit_error(
                                    &global_observability,
                                    "Failed to get remote address",
                                    vec![(
                                        "error.type",
                                        LogAttributeValue::String("tcp_remote_addr_error".into()),
                                    )],
                                );
                                return;
                            };
                            let Ok(local_addr) = socket.local_addr() else {
                                let global_observability =
                                    resolve_root_observability_sink(&server_config.observability_resolver, Some(&ferron_observability::TraceSampler::new(&server_config.trace_sampling)));
                                emit_error(
                                    &global_observability,
                                    "Failed to get local address",
                                    vec![(
                                        "error.type",
                                        LogAttributeValue::String("tcp_local_addr_error".into()),
                                    )],
                                );
                                return;
                            };
                            (remote_addr, local_addr)
                        };
                        let ip_observability = resolve_observability_sink(
                            &server_config.observability_resolver,
                            Some(local_addr.ip()),
                            None,
                            &CompositeEventSink::with_sampler(vec![], Some(TraceSampler::new(&server_config.trace_sampling)))
                        );

                        if let Some(tls_resolver) = &server_config.tls_resolver {
                            let start_handshake = match tokio_rustls::LazyConfigAcceptor::new(Acceptor::default(), socket.into()).await {
                                Ok(start_handshake) => start_handshake,
                                Err(e) => {
                                  emit_error(
                                      &ip_observability,
                                      format!("Failed to start TLS handshake {e}"),
                                      vec![(
                                          "error.type",
                                          LogAttributeValue::String("tcp_tls_handshake_error".into()),
                                      ),
                                      (
                                          "error.message",
                                          LogAttributeValue::String(e.to_string()),
                                      ),
                                      (
                                          "client.address",
                                          LogAttributeValue::String(remote_addr.ip().to_canonical().to_string()),
                                      ),
                                      (
                                          "server.address",
                                          LogAttributeValue::String(local_addr.ip().to_canonical().to_string()),
                                      )],
                                  );
                                  emit_connection_error_metric(&ip_observability, "tcp", "tls_handshake");
                                  return;
                                }
                            };
                            let sni = start_handshake
                                .client_hello()
                                .server_name()
                                .map(std::borrow::ToOwned::to_owned);
                            let hinted_hostname =
                                sni.as_deref().and_then(normalize_host_for_lookup);
                            let connection_options = resolve_http_connection_options(
                                &server_config.http_connection_options_resolver,
                                local_addr.ip(),
                                hinted_hostname.as_deref(),
                            );
                            let resolver = if let Some(sni) = sni.as_deref() {
                                tls_resolver.lookup_ip_and_hostname(local_addr.ip(), sni)
                            } else {
                                tls_resolver.lookup_ip(local_addr.ip())
                            };
                            if let Some(resolver) = resolver {
                                let handshake_start = Instant::now();
                                let tls_stream_option = match
                                    resolver.handshake(start_handshake).await
                                {
                                    Ok(s) => s,
                                    Err(e) => {
                                    let handshake_duration = handshake_start.elapsed();
                                    let host = hinted_hostname.clone().unwrap_or_else(|| "_global".to_string());
                                    let tls_observability = resolve_observability_sink(
                                        &server_config.observability_resolver,
                                        Some(local_addr.ip()),
                                        hinted_hostname.as_deref(),
                                        &ip_observability,
                                    );
                                    let mut error_message = format!("Failed to start TLS handshake: {e}");
                                    let mut attrs = vec![(
                                        "error.type",
                                        LogAttributeValue::String("tcp_tls_handshake_error".into()),
                                    ),
                                    (
                                        "error.message",
                                        LogAttributeValue::String(e.to_string()),
                                    ),
                                    (
                                        "client.address",
                                        LogAttributeValue::String(remote_addr.ip().to_canonical().to_string()),
                                    ),
                                    (
                                        "server.address",
                                        LogAttributeValue::String(local_addr.ip().to_canonical().to_string()),
                                    )];
                                    if e.to_string().to_lowercase().contains("resolve")
                                      || e.to_string().to_lowercase().contains("resolution") {
                                        if let Some(possible_cause) = resolver.get_tls_background_error() {
                                            error_message.push_str(&format!("\nPossible cause: {possible_cause}"));
                                            attrs.push((
                                                "ferron.error.possible_cause",
                                                LogAttributeValue::String(possible_cause.to_string())
                                            ));
                                        }
                                    }
                                    emit_error(
                                        &tls_observability,
                                        error_message,
                                        attrs,
                                    );
                                    emit_connection_error_metric(&tls_observability, "tcp", "tls_handshake");
                                    emit_handshake_duration(
                                        &tls_observability,
                                        &host,
                                        handshake_duration,
                                        "unknown",
                                        "unknown",
                                        "error",
                                    );
                                    emit_handshake_total(&tls_observability, &host, "error");
                                    return;
                                    }
                                };
                                let handshake_duration = handshake_start.elapsed();
                                let host = hinted_hostname.clone().unwrap_or_else(|| "_global".to_string());
                                let tls_observability = resolve_observability_sink(
                                    &server_config.observability_resolver,
                                    Some(local_addr.ip()),
                                    hinted_hostname.as_deref(),
                                    &ip_observability,
                                );
                                if let Some(tls_stream) = tls_stream_option {
                                    let peer_identity = tls_stream
                                        .get_ref()
                                        .1
                                        .peer_certificates()
                                        .filter(|c| !c.is_empty())
                                        .map(|c| c.to_vec());
                                    let negotiated_protocol = tls_stream
                                        .get_ref()
                                        .1
                                        .alpn_protocol()
                                        .map(|protocol| protocol.to_vec());

                                    let protocol_version_str = tls_stream
                                        .get_ref()
                                        .1
                                        .protocol_version()
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("unknown")
                                        .replace('_', ".");
                                    let cipher_suite_str = tls_stream
                                        .get_ref()
                                        .1
                                        .negotiated_cipher_suite()
                                        .map(|cs| cs.suite())
                                        .and_then(|cs| cs.as_str())
                                        .unwrap_or("unknown")
                                        .to_string();

                                    emit_handshake_duration(
                                        &tls_observability,
                                        &host,
                                        handshake_duration,
                                        &protocol_version_str,
                                        &cipher_suite_str,
                                        "success",
                                    );
                                    emit_handshake_total(&tls_observability, &host, "success");
                                    emit_connections_active(&tls_observability, &host, 1);

                                    let tls_params = TlsConnectionParams {
                                        protocol_version: protocol_version_str,
                                        cipher_suite: cipher_suite_str,
                                    };

                                    if negotiated_protocol.as_deref() == Some(b"h2".as_slice()) {
                                        handle_http2_connection(
                                            tls_stream,
                                            ConnectionAddr::Tcp { remote_address: remote_addr, local_address: local_addr },
                                            server_config.pipeline.clone(),
                                            server_config.file_pipeline.clone(),
                                            server_config.error_pipeline.clone(),
                                            server_config.config_resolver.clone(),
                                            hinted_hostname,
                                            true,
                                            server_config.https_port,
                                            connection_options,
                                            server_config.observability_resolver.clone(),
                                            tls_observability.clone(),
                                            (*connection_cancel_token).clone(),
                                            server_config.reload_token.clone(),
                                            http3_alt_svc,
                                            peer_identity,
                                            Some(tls_params),
                                        )
                                        .await;
                                    } else if connection_options.protocols.http1 {
                                        handle_http1_connection(
                                            tls_stream,
                                            ConnectionAddr::Tcp { remote_address: remote_addr, local_address: local_addr },
                                            server_config.pipeline.clone(),
                                            server_config.file_pipeline.clone(),
                                            server_config.error_pipeline.clone(),
                                            server_config.config_resolver.clone(),
                                            hinted_hostname,
                                            true,
                                            server_config.https_port,
                                            connection_options,
                                            server_config.observability_resolver.clone(),
                                            tls_observability.clone(),
                                            (*connection_cancel_token).clone(),
                                            server_config.reload_token.clone(),
                                            http3_alt_svc,
                                            peer_identity,
                                            Some(tls_params),
                                        )
                                        .await;
                                    } else {
                                        emit_error(
                                            &tls_observability,
                                            "TLS connection did not negotiate a supported HTTP protocol",
                                            vec![(
                                                "error.type",
                                                LogAttributeValue::String("tcp_tls_protocol_error".into()),
                                            ),
                                            (
                                                "client.address",
                                                LogAttributeValue::String(remote_addr.ip().to_canonical().to_string()),
                                            ),
                                            (
                                                "server.address",
                                                LogAttributeValue::String(local_addr.ip().to_canonical().to_string()),
                                            )],
                                        );
                                    }

                                    emit_connections_active(&tls_observability, &host, -1);
                                }
                            } else {
                                // Construct empty rustls `ServerConfig`
                                if let Ok(b) = rustls::ServerConfig::builder_with_provider(
                                      Arc::new(rustls::crypto::aws_lc_rs::default_provider())
                                    )
                                    .with_safe_default_protocol_versions() {
                                        let tls_config = b.with_no_client_auth().with_cert_resolver(Arc::new(NoCertResolver));
                                        if let Err(e) = start_handshake.into_stream(Arc::new(tls_config)).await {
                                                                let tls_observability = resolve_observability_sink(
                                                                    &server_config.observability_resolver,
                                                                    Some(local_addr.ip()),
                                                                    hinted_hostname.as_deref(),
                                                                    &ip_observability,
                                                                );
                                                                emit_error(
                                                                    &tls_observability,
                                                                    format!("Failed to start TLS handshake: {e}"),
                                                                    vec![(
                                                                        "error.type",
                                                                        LogAttributeValue::String("tcp_tls_handshake_error".into()),
                                                                    ),
                                                                    (
                                                                        "error.message",
                                                                        LogAttributeValue::String(e.to_string()),
                                                                    ),
                                                                    (
                                                                        "client.address",
                                                                        LogAttributeValue::String(remote_addr.ip().to_canonical().to_string()),
                                                                    ),
                                                                    (
                                                                        "server.address",
                                                                        LogAttributeValue::String(local_addr.ip().to_canonical().to_string()),
                                                                    )],
                                                                );
                                                            }
                                    }
                            }
                        } else {
                            let connection_options = resolve_http_connection_options(
                                &server_config.http_connection_options_resolver,
                                local_addr.ip(),
                                None,
                            );
                            if connection_options.protocols.http2_cleartext {
                            handle_http2_connection(
                                socket,
                                ConnectionAddr::Tcp { remote_address: remote_addr, local_address: local_addr },
                                server_config.pipeline.clone(),
                                server_config.file_pipeline.clone(),
                                server_config.error_pipeline.clone(),
                                server_config.config_resolver.clone(),
                                None,
                                false,
                                server_config.https_port,
                                connection_options,
                                server_config.observability_resolver.clone(),
                                ip_observability,
                                (*connection_cancel_token).clone(),
                                server_config.reload_token.clone(),
                                http3_alt_svc,
                                None,
                                None
                            )
                            .await;
                            } else if connection_options.protocols.http1 {
                            handle_http1_connection_zerocopy(
                                socket,
                                ConnectionAddr::Tcp { remote_address: remote_addr, local_address: local_addr },
                                server_config.pipeline.clone(),
                                server_config.file_pipeline.clone(),
                                server_config.error_pipeline.clone(),
                                server_config.config_resolver.clone(),
                                None,
                                false,
                                server_config.https_port,
                                connection_options,
                                server_config.observability_resolver.clone(),
                                ip_observability,
                                (*connection_cancel_token).clone(),
                                server_config.reload_token.clone(),
                                http3_alt_svc,
                                None
                            )
                            .await;
                            } else {

                                emit_error(
                                    &ip_observability,
                                    "Plain TCP listener requires HTTP/1.x or h2c support",
                                    vec![
                                        (
                                            "error.type",
                                            LogAttributeValue::String("tcp_http1_required".into()),
                                        ),
                                        (
                                            "client.address",
                                            LogAttributeValue::String(remote_addr.ip().to_canonical().to_string()),
                                        ),
                                        (
                                            "client.port",
                                            LogAttributeValue::I64(remote_addr.port() as i64)
                                        ),
                                        (
                                            "server.address",
                                            LogAttributeValue::String(local_addr.ip().to_canonical().to_string()),
                                        ),
                                        (
                                            "server.port",
                                            LogAttributeValue::I64(local_addr.port() as i64)
                                        ),
                                    ],
                                );
                            }
                        }
                    });
                }
            })
        });

        Ok(Self { cancel_token })
    }

    pub fn cancel(&self) {
        self.cancel_token.cancel();
    }
}

fn build_tcp_listener(
    address: SocketAddr,
    tcp_buffer_sizes: (Option<usize>, Option<usize>),
    backlog: Option<i32>,
    multipath: bool,
) -> Result<std::net::TcpListener, io::Error> {
    let domain = if address.is_ipv6() {
        socket2::Domain::IPV6
    } else {
        socket2::Domain::IPV4
    };

    #[cfg(target_os = "linux")]
    let listener_socket = if multipath {
        match socket2::Socket::new(
            domain,
            socket2::Type::STREAM,
            Some(socket2::Protocol::MPTCP),
        ) {
            Ok(s) => {
                log_info!("MPTCP listener enabled on {}", address);
                s
            }
            Err(e) => {
                log_warn!(
                    "MPTCP requested but unavailable ({}), falling back to standard TCP on {}",
                    e,
                    address
                );
                socket2::Socket::new(domain, socket2::Type::STREAM, Some(socket2::Protocol::TCP))?
            }
        }
    } else {
        socket2::Socket::new(domain, socket2::Type::STREAM, Some(socket2::Protocol::TCP))?
    };

    #[cfg(not(target_os = "linux"))]
    let listener_socket = {
        if multipath {
            log_warn!(
                "MPTCP is not supported on this platform, falling back to standard TCP on {}",
                address
            );
        }
        socket2::Socket::new(domain, socket2::Type::STREAM, Some(socket2::Protocol::TCP))?
    };

    listener_socket
        .set_reuse_address(!cfg!(windows))
        .unwrap_or_default();
    if let Some(send_buffer_size) = tcp_buffer_sizes.0 {
        listener_socket
            .set_send_buffer_size(send_buffer_size)
            .unwrap_or_default();
    }
    if let Some(recv_buffer_size) = tcp_buffer_sizes.1 {
        listener_socket
            .set_recv_buffer_size(recv_buffer_size)
            .unwrap_or_default();
    }
    if address.is_ipv6() {
        listener_socket.set_only_v6(false).unwrap_or_default();
    }

    listener_socket.bind(&address.into())?;
    listener_socket.listen(backlog.unwrap_or(-1))?;

    Ok(listener_socket.into())
}
