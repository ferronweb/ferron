use async_trait::async_trait;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use ferron_core::http::HttpContext;
use ferron_module_api::{HttpModule, Module, ProvidesHttp, ProvidesServer, ServerModule};
use ferron_runtime::pipeline::{Pipeline, Stage};

// =====================
// Logging Stage
// =====================

pub struct LoggingStage;

#[async_trait]
impl Stage<HttpContext> for LoggingStage {
    async fn run(&self, ctx: &mut HttpContext) {
        println!("--> {}", ctx.req.uri().path());
    }
}

// =====================
// Hello Handler
// =====================

pub struct HelloStage;

#[async_trait]
impl Stage<HttpContext> for HelloStage {
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

#[async_trait]
impl Stage<HttpContext> for NotFoundStage {
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

pub struct BasicHttpModuleBuilder;

impl HttpModule for BasicHttpModuleBuilder {
    fn register(&self, pipeline: Pipeline<HttpContext>) -> Pipeline<HttpContext> {
        pipeline
            .add_stage(Arc::new(LoggingStage))
            .add_stage(Arc::new(HelloStage))
            .add_stage(Arc::new(NotFoundStage))
    }
}

pub struct BasicHttpModule {
    pipeline: Arc<Pipeline<HttpContext>>,
}

impl BasicHttpModule {
    pub fn new(pipeline: Pipeline<HttpContext>) -> Self {
        Self {
            pipeline: Arc::new(pipeline),
        }
    }
}

impl Module for BasicHttpModule {
    fn name(&self) -> &str {
        "basic_http"
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

impl HttpModule for BasicHttpModule {
    fn register(&self, pipeline: Pipeline<HttpContext>) -> Pipeline<HttpContext> {
        pipeline
            .add_stage(Arc::new(LoggingStage))
            .add_stage(Arc::new(HelloStage))
            .add_stage(Arc::new(NotFoundStage))
    }
}

impl ProvidesHttp for BasicHttpModule {
    fn http(&self) -> Option<&dyn HttpModule> {
        Some(self)
    }
}

impl ProvidesServer for BasicHttpModule {
    fn server(&self) -> Option<&dyn ServerModule> {
        Some(self)
    }
}
