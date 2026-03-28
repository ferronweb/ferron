//! HTTP types and server implementation for Ferron
//!
//! This crate provides HTTP-specific types including HttpContext and
//! ready-to-use HTTP module implementations.

use async_trait::async_trait;
use ferron_common::runtime::Runtime;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use http::{Request, Response};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use ferron_common::pipeline::{Pipeline, Stage};
use ferron_common::Module;
use ferron_common::StageConstraint;

// =====================
// HTTP Context
// =====================

type HttpRequest = Request<Vec<u8>>;
type HttpResponse = Response<Vec<u8>>;

pub struct HttpContext {
    pub req: HttpRequest,
    pub res: HttpResponse,
}

impl HttpContext {
    pub fn new(req: HttpRequest) -> Self {
        Self {
            req,
            res: Response::builder().status(200).body(Vec::new()).unwrap(),
        }
    }
}

// =====================
// Logging Stage
// =====================

pub struct LoggingStage;

impl Default for LoggingStage {
    fn default() -> Self {
        Self
    }
}

#[async_trait]
impl Stage<HttpContext> for LoggingStage {
    fn name(&self) -> &str {
        "logging"
    }

    fn constraints(&self) -> Vec<StageConstraint> {
        vec![StageConstraint::Before("hello".to_string())]
    }

    async fn run(&self, ctx: &mut HttpContext) {
        println!("--> {}", ctx.req.uri().path());
    }
}

// =====================
// Hello Handler
// =====================

pub struct HelloStage;

impl Default for HelloStage {
    fn default() -> Self {
        Self
    }
}

#[async_trait]
impl Stage<HttpContext> for HelloStage {
    fn name(&self) -> &str {
        "hello"
    }

    fn constraints(&self) -> Vec<StageConstraint> {
        vec![StageConstraint::Before("not_found".to_string())]
    }

    async fn run(&self, ctx: &mut HttpContext) {
        if ctx.req.uri().path() == "/" {
            ctx.res = Response::builder()
                .status(200)
                .body(b"Hello from Ferron 3".to_vec())
                .unwrap();
        }
    }
}

// =====================
// 404 Stage
// =====================

pub struct NotFoundStage;

impl Default for NotFoundStage {
    fn default() -> Self {
        Self
    }
}

#[async_trait]
impl Stage<HttpContext> for NotFoundStage {
    fn name(&self) -> &str {
        "not_found"
    }

    async fn run(&self, ctx: &mut HttpContext) {
        if ctx.res.body().is_empty() {
            ctx.res = Response::builder()
                .status(404)
                .body(b"Not Found".to_vec())
                .unwrap();
        }
    }
}

// =====================
// Module Implementation
// =====================

pub struct BasicHttpModule {
    pipeline: Arc<Pipeline<HttpContext>>,
}

impl BasicHttpModule {
    /// Create a new HTTP module with the given pipeline
    pub fn new(pipeline: Pipeline<HttpContext>) -> Self {
        Self {
            pipeline: Arc::new(pipeline),
        }
    }

    /// Create an HTTP module from a registry by building a pipeline from registered stages
    ///
    /// This method retrieves the HTTP stage registry and builds an ordered pipeline
    /// using DAG-based topological sort based on stage constraints (Before/After).
    pub fn from_registry(registry: &ferron_registry::Registry) -> Self {
        let pipeline = registry
            .get_stage_registry::<HttpContext>()
            .expect("HTTP stage registry not found")
            .build_all();
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
                let new_listener = new_listener_result.unwrap();
                new_listener.set_nonblocking(true).unwrap();
                let listener = tokio::net::TcpListener::from_std(new_listener).unwrap();
                loop {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let pipeline = pipeline.clone();

                    tokio::spawn(async move {
                        let mut buf = [0; 1024];
                        let n = socket.read(&mut buf).await.unwrap();

                        if n == 0 {
                            return;
                        }

                        let req_str = String::from_utf8_lossy(&buf[..n]);
                        let path = parse_path(&req_str);

                        let req = Request::builder().uri(path).body(Vec::new()).unwrap();

                        let mut ctx = HttpContext::new(req);

                        pipeline.execute(&mut ctx).await;

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

fn parse_path(req: &str) -> String {
    req.lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/")
        .to_string()
}
