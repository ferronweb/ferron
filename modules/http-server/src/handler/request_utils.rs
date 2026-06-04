use bytes::Bytes;
use ferron_core::config::layer::LayeredConfiguration;
use ferron_core::pipeline::Pipeline;
use ferron_http::{HttpErrorContext, HttpRequest};
use ferron_observability::{
    CompositeEventSink, Event, EventTraceContext, LogEvent, LogLevel, Parent, TraceAttributeValue,
    TraceEvent,
};
use http::{HeaderMap, HeaderValue, Response, StatusCode};
use http_body_util::{BodyExt, Full};
use std::borrow::Cow;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::util::error_pages::generate_default_error_page;

use super::{ResponseBody, LOG_TARGET};

static ERROR_PIPELINE_SPAN_COUNTER: AtomicU64 = AtomicU64::new(1);

#[inline]
pub(super) fn normalize_http2_http3_request(request: &mut HttpRequest) {
    if let Some(authority) = request.uri().authority() {
        let authority_str = authority.as_str();
        // Defense-in-depth: reject empty authority and authority containing control characters
        // to prevent CRLF injection if a non-conformant HTTP/2 parser passes through bad data.
        if !authority_str.is_empty()
            && authority_str
                .bytes()
                .all(|b| b >= 0x20 && b != 0x7F && b != b'\r' && b != b'\n')
        {
            let authority = authority.to_owned();
            let headers = request.headers_mut();
            if let Ok(authority_value) = HeaderValue::from_bytes(authority.as_str().as_bytes()) {
                headers.append(http::header::HOST, authority_value);
            }
        }
    }

    let mut cookie_normalized = String::new();
    let mut cookie_set = false;
    let headers = request.headers_mut();
    for cookie in headers.get_all(http::header::COOKIE) {
        if let Ok(cookie) = cookie.to_str() {
            if cookie_set {
                cookie_normalized.push_str("; ");
            }
            cookie_set = true;
            cookie_normalized.push_str(cookie);
        }
    }
    if cookie_set {
        if let Ok(cookie_value) = HeaderValue::from_bytes(cookie_normalized.as_bytes()) {
            headers.insert(http::header::COOKIE, cookie_value);
        }
    }
}

#[inline]
pub(super) fn normalize_host_header(
    request: &mut HttpRequest,
    _events: &CompositeEventSink,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut host_header_option = request.headers().get_all(http::header::HOST).iter();
    if let Some(header_data) = host_header_option.next() {
        if host_header_option.next().is_some() {
            Err(anyhow::anyhow!("Multiple Host headers found"))?;
        }
        let host_header = header_data.to_str()?.trim();
        let host_header_lower_case = host_header.to_lowercase();
        let host_header_without_dot = host_header_lower_case
            .strip_suffix('.')
            .unwrap_or(host_header_lower_case.as_str());

        if host_header_without_dot != host_header {
            let host_header_value = HeaderValue::from_str(host_header_without_dot)?;
            request
                .headers_mut()
                .insert(http::header::HOST, host_header_value);
        }
    }
    Ok(())
}

#[inline]
pub(super) fn get_http_nested_boolean(
    block: &ferron_core::config::ServerConfigurationBlock,
    directive: &str,
) -> Option<bool> {
    block
        .directives
        .get("http")
        .and_then(|entries| entries.first())
        .and_then(|http_entry| http_entry.children.as_ref())
        .and_then(|http_block| {
            http_block
                .directives
                .get(directive)
                .and_then(|entries| entries.first())
                .and_then(|entry| entry.args.first())
                .and_then(|value| value.as_boolean())
        })
}

#[inline]
pub(super) fn check_backslash_in_path(raw_path: &str, reject: bool) -> Result<(), &'static str> {
    if !reject {
        return Ok(());
    }

    let bytes = raw_path.as_bytes();

    if bytes.contains(&b'\\') {
        return Err("backslash not allowed in URL path");
    }

    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == b'%' && bytes[i + 1].is_ascii_hexdigit() && bytes[i + 2].is_ascii_hexdigit()
        {
            let h1 = bytes[i + 1].to_ascii_lowercase();
            let h2 = bytes[i + 2].to_ascii_lowercase();
            if h1 == b'5' && h2 == b'c' {
                return Err("percent-encoded backslash not allowed in URL path");
            }
            i += 3;
        } else {
            i += 1;
        }
    }

    Ok(())
}

#[inline]
pub(super) fn sanitize_request_url(
    request: &mut HttpRequest,
    decoded_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let url_pathname = request.uri().path();

    if decoded_path != url_pathname {
        let orig_uri = request.uri().clone();
        let mut uri_parts = orig_uri.into_parts();

        let new_path_and_query = format!(
            "{}{}",
            decoded_path,
            uri_parts
                .path_and_query
                .as_ref()
                .and_then(|pq| pq.query())
                .map_or("".to_string(), |q| format!("?{q}"))
        );

        uri_parts.path_and_query = Some(new_path_and_query.parse()?);
        let new_uri = http::Uri::from_parts(uri_parts)?;
        *request.uri_mut() = new_uri;
    }

    Ok(())
}

#[inline]
pub(super) fn is_options_star_request(request: &HttpRequest) -> bool {
    request.method() == http::Method::OPTIONS && request.uri().path() == "*"
}

#[inline]
pub(super) fn builtin_error_response(
    status: u16,
    headers: Option<&HeaderMap>,
    admin_email: Option<String>,
) -> Response<ResponseBody> {
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = generate_default_error_page(status, admin_email.as_deref());
    let mut builder = Response::builder().status(status);
    if let Some(headers) = headers {
        for (name, value) in headers {
            builder = builder.header(name, value);
        }
    }

    builder
        .header(
            http::header::CONTENT_TYPE,
            HeaderValue::from_static("text/html"),
        )
        .body(
            Full::new(Bytes::copy_from_slice(body.as_bytes()))
                .map_err(|e| match e {})
                .boxed_unsync(),
        )
        .unwrap_or_else(|_| builtin_error_response(500, None, admin_email))
}

#[inline]
pub(super) fn emit_error(events: &CompositeEventSink, message: impl Into<String>) {
    emit_error_with_trace(events, message, None);
}

#[inline]
pub(super) fn emit_error_with_trace(
    events: &CompositeEventSink,
    message: impl Into<String>,
    trace_context: Option<EventTraceContext>,
) {
    events.emit(Event::Log(LogEvent {
        level: LogLevel::Error,
        message: message.into(),
        target: LOG_TARGET,
        trace_context,
    }));
}

#[inline]
pub(super) fn emit_warn_with_trace(
    events: &CompositeEventSink,
    message: impl Into<String>,
    trace_context: Option<EventTraceContext>,
) {
    events.emit(Event::Log(LogEvent {
        level: LogLevel::Warn,
        message: message.into(),
        target: LOG_TARGET,
        trace_context,
    }));
}

pub(super) async fn execute_error_pipeline(
    error_pipeline: &Pipeline<HttpErrorContext>,
    error_code: u16,
    headers: Option<HeaderMap>,
    configuration: LayeredConfiguration,
    events: &CompositeEventSink,
    parent_span_key: Option<&str>,
) -> Option<Response<ResponseBody>> {
    let has_traces = events.has_trace_sinks();
    let span_key = has_traces.then(|| {
        format!(
            "error-pipeline:{}",
            ERROR_PIPELINE_SPAN_COUNTER.fetch_add(1, Ordering::Relaxed)
        )
    });

    if let Some(span_key) = span_key.as_ref() {
        events.emit(Event::Trace(TraceEvent::StartSpan {
            key: Cow::Owned(span_key.clone()),
            name: Cow::Borrowed("ferron.pipeline.execute_error"),
            parent: parent_span_key.map(|key| Parent::ByKey(key.to_string())),
            trace_context: None,
            attributes: vec![(
                "http.response.status_code",
                TraceAttributeValue::I64(error_code as i64),
            )],
        }));
    }

    let mut error_ctx = HttpErrorContext {
        error_code,
        headers,
        configuration,
        res: None,
    };

    if let Err(error) = error_pipeline.execute_without_inverse(&mut error_ctx).await {
        emit_error(events, format!("Error pipeline execution error: {error}"));
    }

    if let Some(span_key) = span_key {
        events.emit(Event::Trace(TraceEvent::EndSpan {
            key: Cow::Owned(span_key),
            name: Cow::Borrowed("ferron.pipeline.execute_error"),
            error: None,
            attributes: vec![],
        }));
    }

    error_ctx.res
}

#[inline]
pub(super) fn add_http3_alt_svc_header(headers: &mut HeaderMap, http3_alt_port: Option<u16>) {
    if let Some(http3_alt_port) = http3_alt_port {
        if let Ok(header_value) = match headers.get(http::header::ALT_SVC) {
            Some(value) => {
                let header_value_old = String::from_utf8_lossy(value.as_bytes());
                let header_value_new =
                    format!("h3=\":{http3_alt_port}\", h3-29=\":{http3_alt_port}\"");

                if header_value_old != header_value_new {
                    HeaderValue::from_bytes(
                        format!("{header_value_old}, {header_value_new}").as_bytes(),
                    )
                } else {
                    HeaderValue::from_bytes(header_value_old.as_bytes())
                }
            }
            None => HeaderValue::from_bytes(
                format!("h3=\":{http3_alt_port}\", h3-29=\":{http3_alt_port}\"").as_bytes(),
            ),
        } {
            headers.insert(http::header::ALT_SVC, header_value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_backslash_in_path_rejects_literal_backslash() {
        assert!(check_backslash_in_path("/path\\to\\resource", true).is_err());
        assert!(check_backslash_in_path("/foo\\bar", true).is_err());
        assert!(check_backslash_in_path("\\leading", true).is_err());
        assert!(check_backslash_in_path("/trailing\\", true).is_err());
    }

    #[test]
    fn check_backslash_in_path_rejects_percent_encoded_backslash() {
        assert!(check_backslash_in_path("/path%5Cto%5Cresource", true).is_err());
        assert!(check_backslash_in_path("/path%5cto%5cresource", true).is_err());
        assert!(check_backslash_in_path("/path%5Cto", true).is_err());
        assert!(check_backslash_in_path("/path%5cto", true).is_err());
    }

    #[test]
    fn check_backslash_in_path_allows_normal_paths() {
        assert!(check_backslash_in_path("/", true).is_ok());
        assert!(check_backslash_in_path("/api/v2", true).is_ok());
        assert!(check_backslash_in_path("/users/123", true).is_ok());
        assert!(check_backslash_in_path("/path?query=1", true).is_ok());
    }

    #[test]
    fn normalize_h2_request_adds_host_from_authority() {
        use http_body_util::Empty;

        let mut request = http::Request::builder()
            .version(http::Version::HTTP_2)
            .uri("https://example.com/path")
            .body(Empty::<Bytes>::new().map_err(|e| match e {}).boxed_unsync())
            .unwrap();

        normalize_http2_http3_request(&mut request);

        let host = request.headers().get(http::header::HOST);
        assert!(host.is_some());
        assert_eq!(host.unwrap().to_str().unwrap(), "example.com");
    }

    #[test]
    fn normalize_h2_request_skips_empty_authority() {
        use http_body_util::Empty;

        // Authority-less URI (shouldn't happen in practice, but tests defense-in-depth)
        let mut request = http::Request::builder()
            .version(http::Version::HTTP_2)
            .uri("/path-only")
            .body(Empty::<Bytes>::new().map_err(|e| match e {}).boxed_unsync())
            .unwrap();

        normalize_http2_http3_request(&mut request);

        // No Host header should be added from authority when authority is absent
        assert!(!request.headers().contains_key(http::header::HOST));
    }

    #[test]
    fn normalize_h2_request_preserves_existing_host_header() {
        use http_body_util::Empty;

        let mut request = http::Request::builder()
            .version(http::Version::HTTP_2)
            .uri("https://authority.example.com/path")
            .header(http::header::HOST, "original.example.com")
            .body(Empty::<Bytes>::new().map_err(|e| match e {}).boxed_unsync())
            .unwrap();

        normalize_http2_http3_request(&mut request);

        // Host header should have the authority value appended (since we use append)
        let hosts: Vec<_> = request
            .headers()
            .get_all(http::header::HOST)
            .iter()
            .collect();
        assert_eq!(hosts.len(), 2);
    }
}
