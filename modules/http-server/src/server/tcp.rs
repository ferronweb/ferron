//! TCP listener and connection handling

use std::net::{Ipv6Addr, SocketAddr};
use std::sync::Arc;

use ferron_core::providers::Provider;
use ferron_core::runtime::Runtime;
use ferron_observability::{
    CompositeEventSink, Event, EventSink, LogEvent, LogLevel, ObservabilityContext,
    ObservabilityProviderEventSink,
};
use http::Request;
use http_body_util::BodyExt;
use rustls::server::Acceptor;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use ferron_core::pipeline::Pipeline;
use ferron_core::{log_error, log_info};
use ferron_http::HttpContext;

use crate::server::tls_resolve::{RadixTree, TlsResolverRadixTree};

pub struct TcpListenerHandle {
    cancel_token: Arc<tokio_util::sync::CancellationToken>,
}

impl TcpListenerHandle {
    pub fn new(
        port: u16,
        pipeline: Arc<Pipeline<HttpContext>>,
        runtime: &mut Runtime,
        tls_resolver: Option<Arc<TlsResolverRadixTree>>,
        observability_resolver: Arc<super::tls_resolve::RadixTree<Vec<Arc<dyn EventSink>>>>,
    ) -> Result<Self, std::io::Error> {
        // TODO: listen address
        let addr = SocketAddr::from((Ipv6Addr::UNSPECIFIED, port));
        let listener = std::net::TcpListener::bind(addr)?;

        log_info!("HTTP server listening on {}", addr);

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
                if let Err(e) = new_listener.set_nonblocking(true) {
                    log_error!("Failed to set listener non-blocking: {}", e);
                    return;
                }
                let Ok(listener) = vibeio::net::TcpListener::from_std(new_listener) else {
                    log_error!("Failed to convert listener to tokio");
                    return;
                };
                loop {
                    let accept_result = tokio::select! {
                        res = listener.accept() => res,
                        _ = cancel_token.cancelled() => {
                            return;
                        }
                    };
                    let Ok((socket, _)) = accept_result else {
                        global_observability.emit(Event::Log(LogEvent {
                            level: LogLevel::Error,
                            message: "Failed to accept connection".to_string(),
                            target: "ferron-http-server",
                        }));
                        continue;
                    };
                    let _ = socket.set_nodelay(true);
                    let Ok(socket) = socket.into_poll() else {
                        global_observability.emit(Event::Log(LogEvent {
                            level: LogLevel::Error,
                            message: "Failed to convert socket to poll-based I/O".to_string(),
                            target: "ferron-http-server",
                        }));
                        return;
                    };

                    let pipeline = pipeline.clone();
                    let tls_resolver = tls_resolver.clone();
                    let global_observability = global_observability.clone();
                    vibeio::spawn(async move {
                        if let Some(tls_resolver) = tls_resolver {
                            let Ok(local_addr) = socket.local_addr() else {
                                global_observability.emit(Event::Log(LogEvent {
                                    level: LogLevel::Error,
                                    message: "Failed to get local address".to_string(),
                                    target: "ferron-http-server",
                                }));
                                return;
                            };
                            let Ok(start_handshake) =
                                tokio_rustls::LazyConfigAcceptor::new(Acceptor::default(), socket)
                                    .await
                            else {
                                global_observability.emit(Event::Log(LogEvent {
                                    level: LogLevel::Error,
                                    message: "Failed to start TLS handshake".to_string(),
                                    target: "ferron-http-server",
                                }));
                                return;
                            };
                            let resolver =
                                if let Some(sni) = start_handshake.client_hello().server_name() {
                                    tls_resolver.lookup_ip_and_hostname(local_addr.ip(), sni)
                                } else {
                                    tls_resolver.lookup_ip(local_addr.ip())
                                };
                            if let Some(resolver) = resolver {
                                let Ok(tls_stream_option) =
                                    resolver.handshake(start_handshake).await
                                else {
                                    global_observability.emit(Event::Log(LogEvent {
                                        level: LogLevel::Error,
                                        message: "Failed to start TLS handshake".to_string(),
                                        target: "ferron-http-server",
                                    }));
                                    return;
                                };
                                // TODO: per IP and host observability
                                if let Some(tls_stream) = tls_stream_option {
                                    handle_http(tls_stream, pipeline).await;
                                }
                            }
                        } else {
                            handle_http(socket, pipeline).await;
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

async fn handle_http<S>(mut socket: S, pipeline: Arc<Pipeline<HttpContext>>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut buf = [0; 1024];
    let Ok(n) = socket.read(&mut buf).await else {
        log_error!("Failed to read from socket");
        return;
    };

    if n == 0 {
        return;
    }

    let req_str = String::from_utf8_lossy(&buf[..n]);
    let path = parse_path(&req_str);

    let Ok(req) = Request::builder().uri(path).body(
        http_body_util::Empty::<bytes::Bytes>::new()
            .map_err(|e| match e {})
            .boxed_unsync(),
    ) else {
        log_error!("Failed to build request");
        return;
    };

    let mut ctx = HttpContext {
        events: ferron_observability::CompositeEventSink::new(vec![]),
        req: Some(req),
        res: None,
        variables: std::collections::HashMap::new(),
    };

    if let Err(e) = pipeline.execute(&mut ctx).await {
        log_error!("Pipeline execution error: {}", e);
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
    let _ = socket.write_all(response.as_bytes()).await;
    let mut body = res.into_body();
    while let Some(chunk_result) = body.frame().await {
        let Some(chunk) = chunk_result.ok().and_then(|c| c.into_data().ok()) else {
            log_error!("Failed to read response body chunk");
            break;
        };
        if let Err(e) = socket.write_all(&chunk).await {
            log_error!("Failed to write response body chunk: {}", e);
            break;
        }
    }
}
fn parse_path(req: &str) -> String {
    req.lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/")
        .to_string()
}
