use std::collections::HashMap;
use std::io;
use std::net::IpAddr;
use std::sync::Arc;

use bytes::Bytes;
use ferron_core::pipeline::Pipeline;
use ferron_http::{HttpContext, HttpRequest, HttpResponse};
use ferron_observability::{CompositeEventSink, Event, LogEvent, LogLevel};
use http::{HeaderMap, Response, StatusCode};
use http_body_util::{combinators::UnsyncBoxBody, BodyExt, Full};

use crate::config::ThreeStageResolver;

const LOG_TARGET: &str = "ferron-http-server";
type ResponseBody = UnsyncBoxBody<Bytes, io::Error>;

pub async fn request_handler(
    request: HttpRequest,
    pipeline: Arc<Pipeline<HttpContext>>,
    config_resolver: Arc<ThreeStageResolver>,
    local_ip: IpAddr,
    hostname: Option<String>,
    is_tls: bool,
    events: CompositeEventSink,
) -> Result<Response<ResponseBody>, io::Error> {
    // TODO: remodel the pipeline handler
    let mut variables = HashMap::new();
    if let Some(hostname) = hostname.as_ref() {
        variables.insert("request.host".to_string(), hostname.clone());
    }
    variables.insert(
        "request.scheme".to_string(),
        if is_tls { "https" } else { "http" }.to_string(),
    );
    variables.insert("server.ip".to_string(), local_ip.to_string());

    let resolver_request = build_resolver_request(&request)?;
    let resolution = config_resolver.resolve(
        local_ip,
        hostname.as_deref().unwrap_or(""),
        request.uri().path(),
        &(resolver_request, variables.clone()),
    );

    let Some(resolution) = resolution else {
        return Ok(text_response(StatusCode::NOT_FOUND, b"Not Found"));
    };

    let mut ctx = HttpContext {
        req: Some(request),
        res: None,
        events: events.clone(),
        configuration: resolution.configuration,
        hostname,
        variables,
    };

    if let Err(error) = pipeline.execute(&mut ctx).await {
        emit_error(&events, format!("Pipeline execution error: {error}"));
        return Ok(text_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            b"Internal Server Error",
        ));
    }

    Ok(
        match ctx.res.unwrap_or(HttpResponse::BuiltinError(500, None)) {
            HttpResponse::Custom(response) => response,
            HttpResponse::BuiltinError(status, headers) => {
                builtin_error_response(status, headers.as_ref())
            }
            HttpResponse::Abort => empty_response(StatusCode::NO_CONTENT),
        },
    )
}

fn build_resolver_request(request: &HttpRequest) -> Result<HttpRequest, io::Error> {
    let mut builder = http::Request::builder()
        .method(request.method().clone())
        .uri(request.uri().clone())
        .version(request.version());
    for (name, value) in request.headers() {
        builder = builder.header(name, value);
    }

    builder
        .body(
            http_body_util::Empty::<Bytes>::new()
                .map_err(|e| match e {})
                .boxed_unsync(),
        )
        .map_err(|error| io::Error::other(error.to_string()))
}

fn builtin_error_response(status: u16, headers: Option<&HeaderMap>) -> Response<ResponseBody> {
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = status.canonical_reason().unwrap_or("Error");
    let mut builder = Response::builder().status(status);
    if let Some(headers) = headers {
        for (name, value) in headers {
            builder = builder.header(name, value);
        }
    }

    builder
        .body(
            Full::new(Bytes::copy_from_slice(body.as_bytes()))
                .map_err(|e| match e {})
                .boxed_unsync(),
        )
        .unwrap_or_else(|_| {
            text_response(StatusCode::INTERNAL_SERVER_ERROR, b"Internal Server Error")
        })
}

fn empty_response(status: StatusCode) -> Response<ResponseBody> {
    Response::builder()
        .status(status)
        .body(
            http_body_util::Empty::<Bytes>::new()
                .map_err(|e| match e {})
                .boxed_unsync(),
        )
        .expect("failed to build empty response")
}

fn text_response(status: StatusCode, body: &'static [u8]) -> Response<ResponseBody> {
    Response::builder()
        .status(status)
        .body(
            Full::new(Bytes::from_static(body))
                .map_err(|e| match e {})
                .boxed_unsync(),
        )
        .expect("failed to build text response")
}

fn emit_error(events: &CompositeEventSink, message: impl Into<String>) {
    events.emit(Event::Log(LogEvent {
        level: LogLevel::Error,
        message: message.into(),
        target: LOG_TARGET,
    }));
}
