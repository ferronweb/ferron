//! TCP listener and connection handling

use std::net::{Ipv6Addr, SocketAddr};
use std::sync::Arc;

use http::Request;
use http_body_util::BodyExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use ferron_core::pipeline::Pipeline;
use ferron_core::{log_error, log_info};
use ferron_http::HttpContext;

pub struct TcpListenerHandle {
    cancel_token: Arc<tokio_util::sync::CancellationToken>,
}

impl TcpListenerHandle {
    pub fn new(port: u16, pipeline: Arc<Pipeline<HttpContext>>) -> Result<Self, std::io::Error> {
        // TODO: listen address
        let addr = SocketAddr::from((Ipv6Addr::UNSPECIFIED, port));
        let listener = std::net::TcpListener::bind(addr)?;

        log_info!("HTTP server listening on {}", addr);

        let cancel_token = Arc::new(tokio_util::sync::CancellationToken::new());

        let pipeline_clone = pipeline.clone();
        let cancel_token_clone = cancel_token.clone();

        // TODO: replace with proper HTTP server implementation
        //       use RELOAD_TOKEN from the core for config reloads
        vibeio::spawn(async move {
            let new_listener_result = listener.try_clone();
            let pipeline = pipeline_clone.clone();
            let cancel_token = cancel_token_clone.clone();
            Box::pin(async move {
                let Ok(new_listener) = new_listener_result else {
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
                        log_error!("Failed to accept connection");
                        continue;
                    };
                    let pipeline = pipeline.clone();

                    let _ = socket.set_nodelay(true);
                    let Ok(mut socket) = socket.into_poll() else {
                        log_error!("Failed to convert socket to poll-based I/O");
                        continue;
                    };

                    vibeio::spawn(async move {
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
                                        http_body_util::Full::new(bytes::Bytes::from_static(
                                            b"Internal Server Error",
                                        ))
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
                            let Some(chunk) = chunk_result.ok().and_then(|c| c.into_data().ok())
                            else {
                                log_error!("Failed to read response body chunk");
                                break;
                            };
                            if let Err(e) = socket.write_all(&chunk).await {
                                log_error!("Failed to write response body chunk: {}", e);
                                break;
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

fn parse_path(req: &str) -> String {
    req.lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/")
        .to_string()
}
