//! TCP listener and connection handling

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use ferron_core::runtime::Runtime;
use ferron_observability::{CompositeEventSink, Event, EventSink, LogEvent, LogLevel};
use http::Request;
use http_body_util::BodyExt;
use rustls::server::Acceptor;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use ferron_core::pipeline::Pipeline;
use ferron_core::{log_error, log_info};
use ferron_http::HttpContext;

use crate::server::tls_resolve::{RadixTree, TlsResolverRadixTree};

const LOG_TARGET: &str = "ferron-http-server";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TcpListenerOptions {
    pub address: SocketAddr,
    pub send_buffer_size: Option<usize>,
    pub recv_buffer_size: Option<usize>,
}

pub struct TcpListenerHandle {
    cancel_token: Arc<tokio_util::sync::CancellationToken>,
}

impl TcpListenerHandle {
    pub fn new(
        options: TcpListenerOptions,
        pipeline: Arc<Pipeline<HttpContext>>,
        runtime: &mut Runtime,
        tls_resolver: Option<Arc<TlsResolverRadixTree>>,
        observability_resolver: Arc<super::tls_resolve::RadixTree<Vec<Arc<dyn EventSink>>>>,
    ) -> Result<Self, std::io::Error> {
        let listener = build_tcp_listener(
            options.address,
            (options.send_buffer_size, options.recv_buffer_size),
        )?;

        log_info!("HTTP server listening on {}", options.address);

        let cancel_token = Arc::new(tokio_util::sync::CancellationToken::new());

        let pipeline_clone = pipeline.clone();
        let cancel_token_clone = cancel_token.clone();

        // TODO: replace with proper HTTP server implementation
        //       use RELOAD_TOKEN from the core for config reloads
        runtime.spawn_primary_task(move || {
            let new_listener_result = listener.try_clone();
            let cancel_token = cancel_token_clone.clone();
            let tls_resolver = tls_resolver.clone();
            let observability_resolver = observability_resolver.clone();
            let global_observability = observability_resolver
                .root_data()
                .map(CompositeEventSink::new)
                .unwrap_or(CompositeEventSink::new(vec![]));
            let pipeline = pipeline_clone.clone();
            Box::pin(async move {
                let Ok(new_listener) = new_listener_result else {
                    // TODO: logging providers
                    log_error!("Failed to clone listener");
                    return;
                };
                let Ok(listener) = vibeio::net::TcpListener::from_std(new_listener) else {
                    log_error!("Failed to convert listener to vibeio");
                    return;
                };
                #[cfg(unix)]
                let mut handle_exhaustion_backoff = Duration::from_millis(10);
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
                                handle_exhaustion_backoff = Duration::from_millis(10);
                            }
                            socket
                        }
                        Err(err) => {
                            emit_error(
                                &global_observability,
                                format!("Failed to accept connection: {}", err),
                            );
                            #[cfg(unix)]
                            if err.raw_os_error() == Some(24) {
                                vibeio::time::sleep(handle_exhaustion_backoff).await;
                                handle_exhaustion_backoff =
                                    handle_exhaustion_backoff.saturating_mul(2);
                                if handle_exhaustion_backoff > Duration::from_secs(1) {
                                    handle_exhaustion_backoff = Duration::from_secs(1);
                                }
                            }
                            continue;
                        }
                    };
                    let _ = socket.set_nodelay(true);
                    let Ok(socket) = socket.into_poll() else {
                        emit_error(
                            &global_observability,
                            "Failed to convert socket to poll-based I/O",
                        );
                        continue;
                    };

                    let pipeline = pipeline.clone();
                    let tls_resolver = tls_resolver.clone();
                    let observability_resolver = observability_resolver.clone();
                    let global_observability = global_observability.clone();
                    vibeio::spawn(async move {
                        let Ok(local_addr) = socket.local_addr() else {
                            emit_error(&global_observability, "Failed to get local address");
                            return;
                        };
                        let ip_observability = resolve_observability_sink(
                            &observability_resolver,
                            Some(local_addr.ip()),
                            None,
                            &global_observability,
                        );

                        if let Some(tls_resolver) = tls_resolver {
                            let Ok(start_handshake) =
                                tokio_rustls::LazyConfigAcceptor::new(Acceptor::default(), socket)
                                    .await
                            else {
                                emit_error(&ip_observability, "Failed to start TLS handshake");
                                return;
                            };
                            let sni = start_handshake
                                .client_hello()
                                .server_name()
                                .map(std::borrow::ToOwned::to_owned);
                            let resolver = if let Some(sni) = sni.as_deref() {
                                tls_resolver.lookup_ip_and_hostname(local_addr.ip(), sni)
                            } else {
                                tls_resolver.lookup_ip(local_addr.ip())
                            };
                            if let Some(resolver) = resolver {
                                let Ok(tls_stream_option) =
                                    resolver.handshake(start_handshake).await
                                else {
                                    let tls_observability = resolve_observability_sink(
                                        &observability_resolver,
                                        Some(local_addr.ip()),
                                        sni.as_deref(),
                                        &ip_observability,
                                    );
                                    emit_error(&tls_observability, "Failed to start TLS handshake");
                                    return;
                                };
                                let tls_observability = resolve_observability_sink(
                                    &observability_resolver,
                                    Some(local_addr.ip()),
                                    sni.as_deref(),
                                    &ip_observability,
                                );
                                if let Some(tls_stream) = tls_stream_option {
                                    handle_http(
                                        tls_stream,
                                        pipeline,
                                        local_addr.ip(),
                                        sni,
                                        observability_resolver,
                                        tls_observability,
                                    )
                                    .await;
                                }
                            }
                        } else {
                            handle_http(
                                socket,
                                pipeline,
                                local_addr.ip(),
                                None,
                                observability_resolver,
                                ip_observability,
                            )
                            .await;
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
) -> Result<std::net::TcpListener, io::Error> {
    let listener_socket = socket2::Socket::new(
        if address.is_ipv6() {
            socket2::Domain::IPV6
        } else {
            socket2::Domain::IPV4
        },
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )?;

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
    listener_socket.listen(1024)?;

    Ok(listener_socket.into())
}

async fn handle_http<S>(
    mut socket: S,
    pipeline: Arc<Pipeline<HttpContext>>,
    local_ip: IpAddr,
    hinted_hostname: Option<String>,
    observability_resolver: Arc<RadixTree<Vec<Arc<dyn EventSink>>>>,
    default_observability: CompositeEventSink,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut buf = [0; 1024];
    let Ok(n) = socket.read(&mut buf).await else {
        emit_error(&default_observability, "Failed to read from socket");
        return;
    };

    if n == 0 {
        return;
    }

    let req_str = String::from_utf8_lossy(&buf[..n]);
    let host = parse_host(&req_str).or(hinted_hostname);
    let request_observability = resolve_observability_sink(
        &observability_resolver,
        Some(local_ip),
        host.as_deref(),
        &default_observability,
    );
    let path = parse_path(&req_str);

    let Ok(req) = Request::builder().uri(path).body(
        http_body_util::Empty::<bytes::Bytes>::new()
            .map_err(|e| match e {})
            .boxed_unsync(),
    ) else {
        emit_error(&request_observability, "Failed to build request");
        return;
    };

    let mut ctx = HttpContext {
        events: request_observability.clone(),
        req: Some(req),
        res: None,
        variables: std::collections::HashMap::new(),
    };

    if let Err(e) = pipeline.execute(&mut ctx).await {
        emit_error(
            &request_observability,
            format!("Pipeline execution error: {}", e),
        );
    }

    let ferron_http::HttpResponse::Custom(res) = ctx.res.unwrap_or_else(|| {
        ferron_http::HttpResponse::Custom(
            http::Response::builder()
                .status(500)
                .body(
                    http_body_util::Full::new(bytes::Bytes::from_static(b"Internal Server Error"))
                        .map_err(|e| match e {})
                        .boxed_unsync(),
                )
                .expect("Failed to build 500 response"),
        )
    }) else {
        todo!("Handle non-custom responses (e.g. built-in errors, aborts)");
    };

    let response = format!("HTTP/1.1 {} OK\r\n\r\n", res.status().as_u16());
    if let Err(e) = socket.write_all(response.as_bytes()).await {
        emit_error(
            &request_observability,
            format!("Failed to write response head: {}", e),
        );
        return;
    }
    let mut body = res.into_body();
    while let Some(chunk_result) = body.frame().await {
        let Some(chunk) = chunk_result.ok().and_then(|c| c.into_data().ok()) else {
            emit_error(&request_observability, "Failed to read response body chunk");
            break;
        };
        if let Err(e) = socket.write_all(&chunk).await {
            emit_error(
                &request_observability,
                format!("Failed to write response body chunk: {}", e),
            );
            break;
        }
    }
}

fn resolve_observability_sink(
    observability_resolver: &RadixTree<Vec<Arc<dyn EventSink>>>,
    ip: Option<IpAddr>,
    hostname: Option<&str>,
    fallback: &CompositeEventSink,
) -> CompositeEventSink {
    let normalized_hostname = hostname.and_then(normalize_host_for_lookup);
    let sinks = match (ip, normalized_hostname.as_deref()) {
        (Some(ip), Some(hostname)) => observability_resolver.lookup_ip_and_hostname(ip, hostname),
        (Some(ip), None) => observability_resolver.lookup_ip(ip),
        (None, Some(hostname)) => observability_resolver.lookup_hostname(hostname),
        (None, None) => observability_resolver.root_data(),
    };

    sinks
        .map(CompositeEventSink::new)
        .unwrap_or_else(|| fallback.clone())
}

fn emit_error(observability: &CompositeEventSink, message: impl Into<String>) {
    observability.emit(Event::Log(LogEvent {
        level: LogLevel::Error,
        message: message.into(),
        target: LOG_TARGET,
    }));
}

fn parse_path(req: &str) -> String {
    req.lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/")
        .to_string()
}

fn parse_host(req: &str) -> Option<String> {
    for line in req.lines().skip(1) {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("host") {
            return normalize_host_for_lookup(value);
        }
    }
    None
}

fn normalize_host_for_lookup(host: &str) -> Option<String> {
    let host = host.trim();
    if host.is_empty() {
        return None;
    }

    if let Some(rest) = host.strip_prefix('[') {
        let end = rest.find(']')?;
        return Some(rest[..end].to_ascii_lowercase());
    }

    let host_without_port = match host.rsplit_once(':') {
        Some((candidate, port))
            if !candidate.contains(':') && port.chars().all(|c| c.is_ascii_digit()) =>
        {
            candidate
        }
        _ => host,
    };
    let normalized = host_without_port.trim().trim_end_matches('.');
    if normalized.is_empty() {
        return None;
    }

    Some(normalized.to_ascii_lowercase())
}
