#![no_main]

use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use ferron_core::pipeline::Pipeline;
use ferron_http::{HttpContext, HttpErrorContext, HttpFileContext};
use ferron_http_server::handler::request_handler;
use ferron_http_server::config::ThreeStageResolver;
use ferron_http_server::stages::{ClientIpFromHeaderStage, HttpsRedirectStage};
use ferron_observability::CompositeEventSink;
use http_body_util::BodyExt;
use libfuzzer_sys::fuzz_target;

/// Parse an HTTP request from raw bytes using httparse.
///
/// Format (first byte selects mode):
///   0x00 = plain HTTP/1.1 request
///   0x01 = HTTP/2-style request (with :authority pseudo-header)
///
/// Remaining bytes are treated as raw HTTP/1 request text.
fn parse_http_request(input: &[u8]) -> Option<http::Request<http_body_util::combinators::UnsyncBoxBody<Bytes, std::io::Error>>> {
    if input.is_empty() {
        return None;
    }

    let mode = input[0];
    let data = &input[1..];

    if data.is_empty() {
        return None;
    }

    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut req = httparse::Request::new(&mut headers);

    match req.parse(data) {
        Ok(httparse::Status::Complete(_)) => {}
        _ => return None,
    }

    let method = req.method?;
    let path = req.path?;

    let mut builder = http::Request::builder()
        .method(method)
        .uri(path)
        .version(http::Version::HTTP_11);

    // For HTTP/2-style mode, add :authority as Host header
    if mode == 0x01 {
        if let Some(authority) = req.path.and_then(|p| {
            if p.starts_with("http://") || p.starts_with("https://") {
                p.split("://").nth(1)?.split('/').next()
            } else {
                None
            }
        }) {
            if let Ok(val) = http::HeaderValue::from_str(authority) {
                builder = builder.header(http::header::HOST, val);
            }
        }
    }

    for header in req.headers.iter() {
        if let (Ok(name), Ok(value)) = (
            http::HeaderName::from_bytes(header.name.as_bytes()),
            http::HeaderValue::from_bytes(header.value),
        ) {
            builder = builder.header(name, value);
        }
    }

    let body = http_body_util::Empty::<Bytes>::new()
        .map_err(|e| -> std::io::Error { match e {} })
        .boxed_unsync();
    Some(builder.body(body).ok()?)
}

fuzz_target!(|input: &[u8]| {
    let Some(request) = parse_http_request(input) else {
        return;
    };

    // Build the same minimal pipeline that the real server uses
    let pipeline: Arc<Pipeline<HttpContext>> = Arc::new(
        Pipeline::new()
            .add_stage(Arc::new(ClientIpFromHeaderStage))
            .add_stage(Arc::new(HttpsRedirectStage)),
    );
    let file_pipeline: Arc<Pipeline<HttpFileContext>> = Arc::new(Pipeline::new());
    let error_pipeline: Arc<Pipeline<HttpErrorContext>> = Arc::new(Pipeline::new());
    let resolver: Arc<ThreeStageResolver> = Arc::new(ThreeStageResolver::new());
    let events = CompositeEventSink::new(vec![]);

    let local_address: SocketAddr = "127.0.0.1:80".parse().unwrap();
    let remote_address: SocketAddr = "127.0.0.1:12345".parse().unwrap();

    // Use a tokio runtime to execute the async handler
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async {
        let _result = request_handler(
            request,
            pipeline,
            file_pipeline,
            error_pipeline,
            resolver,
            local_address,
            remote_address,
            Some("localhost".to_string()),
            false,  // encrypted
            false,  // http3_alt_svc
            None,   // https_port
            events,
            None,   // timeout_duration
            None,   // peer_identity
        )
        .await;

        // The handler must not panic for any valid HTTP request.
        // Errors (400, 404, 500, etc.) are expected and returned as responses.
    });
});
