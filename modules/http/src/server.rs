//! HTTP server implementation

use std::sync::Arc;

use http::Request;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use ferron_core::Module;
use ferron_core::pipeline::Pipeline;
use ferron_core::runtime::Runtime;

use crate::context::HttpContext;

pub struct BasicHttpModule {
    pipeline: Arc<Pipeline<HttpContext>>,
}

impl BasicHttpModule {
    /// Create an HTTP module from a registry and configuration by building a pipeline from registered stages
    ///
    /// This method retrieves the HTTP stage registry and builds an ordered pipeline
    /// using DAG-based topological sort based on stage constraints (Before/After).
    ///
    /// # Panics
    ///
    /// Panics if the HTTP stage registry is not found. This should only happen
    /// if the registry was not properly initialized with HTTP stages.
    pub fn new(
        registry: &ferron_core::registry::Registry,
        port_config: ferron_core::config::ServerConfigurationPort,
        global_config: Arc<ferron_core::config::ServerConfigurationBlock>,
    ) -> Self {
        let pipeline = registry
            .get_stage_registry::<HttpContext>()
            .expect("HTTP stage registry not found")
            .build_all();
        // TODO: process configuration
        Self {
            pipeline: Arc::new(pipeline),
        }
    }
}

impl Module for BasicHttpModule {
    fn name(&self) -> &str {
        "http"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn start(&self, runtime: &mut Runtime) -> Result<(), Box<dyn std::error::Error>> {
        let pipeline = self.pipeline.clone();

        let listener = std::net::TcpListener::bind("127.0.0.1:8080")?;

        println!("HTTP server listening on 127.0.0.1:8080");

        runtime.spawn_primary_task(move || {
            let new_listener_result = listener.try_clone();
            let pipeline = pipeline.clone();
            Box::pin(async move {
                let Ok(new_listener) = new_listener_result else {
                    eprintln!("Failed to clone listener");
                    return;
                };
                if let Err(e) = new_listener.set_nonblocking(true) {
                    eprintln!("Failed to set listener non-blocking: {}", e);
                    return;
                }
                let Ok(listener) = tokio::net::TcpListener::from_std(new_listener) else {
                    eprintln!("Failed to convert listener to tokio");
                    return;
                };
                loop {
                    let Ok((mut socket, _)) = listener.accept().await else {
                        eprintln!("Failed to accept connection");
                        continue;
                    };
                    let pipeline = pipeline.clone();

                    tokio::spawn(async move {
                        let mut buf = [0; 1024];
                        let Ok(n) = socket.read(&mut buf).await else {
                            eprintln!("Failed to read from socket");
                            return;
                        };

                        if n == 0 {
                            return;
                        }

                        let req_str = String::from_utf8_lossy(&buf[..n]);
                        let path = parse_path(&req_str);

                        let Ok(req) = Request::builder().uri(path).body(Vec::new()) else {
                            eprintln!("Failed to build request");
                            return;
                        };

                        let mut ctx = HttpContext::new(req);

                        if let Err(e) = pipeline.execute(&mut ctx).await {
                            eprintln!("Pipeline execution error: {}", e);
                        }

                        let res = ctx.res;

                        let response = format!(
                            "HTTP/1.1 {} OK\r\nContent-Length: {}\r\n\r\n{}",
                            res.status().as_u16(),
                            res.body().len(),
                            String::from_utf8_lossy(res.body())
                        );

                        let _ = socket.write_all(response.as_bytes()).await;
                    });
                }
            })
        });

        Ok(())
    }
}

// TODO: cancel HTTP task when module is dropped.

fn parse_path(req: &str) -> String {
    req.lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/")
        .to_string()
}
