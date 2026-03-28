use async_trait::async_trait;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use ferron_core::http::HttpContext;
use ferron_module_api::{Module, ProvidesServer, ServerModule};
use ferron_runtime::pipeline::{Pipeline, Stage};
use ferron_runtime::StageConstraint;

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
            ctx.res = http::Response::builder()
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
            ctx.res = http::Response::builder()
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
    pub fn from_registry(registry: &ferron_registry::Registry) -> Self {
        let pipeline = registry
            .get_stage_registry::<ferron_core::http::HttpContext>()
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
}

impl ServerModule for BasicHttpModule {
    fn start(&self) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        let pipeline = self.pipeline.clone();

        Box::pin(async move {
            let listener = TcpListener::bind("127.0.0.1:8080").await.unwrap();

            println!("HTTP server listening on 127.0.0.1:8080");

            loop {
                // TODO: proper HTTP implementation
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

                    let req = http::Request::builder().uri(path).body(Vec::new()).unwrap();

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
    }
}

fn parse_path(req: &str) -> String {
    req.lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/")
        .to_string()
}

impl ProvidesServer for BasicHttpModule {
    fn server(&self) -> Option<&dyn ServerModule> {
        Some(self)
    }
}
