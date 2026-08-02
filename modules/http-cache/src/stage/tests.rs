use super::*;
use ferron_core::config::layer::LayeredConfiguration;
use ferron_observability::CompositeEventSink;
use http::Request;
use std::net::SocketAddr;

use super::key::{build_base_key, build_private_cache_key};

#[inline]
fn test_context(path: &str) -> HttpContext {
    let request = Request::builder()
        .uri(path)
        .header(http::header::HOST, "example.com")
        .body(
            http_body_util::Empty::<bytes::Bytes>::new()
                .map_err(|error: std::convert::Infallible| match error {})
                .boxed_unsync(),
        )
        .unwrap();

    HttpContext {
        req: Some(request),
        res: None,
        events: CompositeEventSink::new(Vec::new()),
        configuration: LayeredConfiguration::default(),
        hostname: Some("example.com".to_string()),
        variables: rustc_hash::FxHashMap::default(),
        previous_error: None,
        original_uri: None,
        routing_uri: None,
        encrypted: true,
        local_address: "127.0.0.1:443".parse::<SocketAddr>().unwrap(),
        remote_address: "127.0.0.2:12345".parse::<SocketAddr>().unwrap(),
        auth_user: None,
        https_port: Some(443),
        extensions: typemap_rev::TypeMap::new(),
    }
}

#[test]
fn parses_private_key_from_cookies() {
    let mut cookies = ahash::AHashMap::default();
    cookies.insert("PHPSESSID".to_string(), "1234567890abcdef".to_string());
    let key = build_private_cache_key(&cookies, "127.0.0.1".parse().unwrap(), Some("user"));
    assert!(key.contains("auth=user"));
    assert!(key.contains("cookie:PHPSESSID=1234567890abcdef"));
}

#[test]
fn base_key_uses_scheme_host_and_path() {
    let ctx = test_context("/test?q=1");
    let request = ctx.req.as_ref().unwrap();
    let key = build_base_key(ctx.encrypted, request.headers(), None, request.uri());
    assert_eq!(key, "https://example.com/test?q=1");
}

#[test]
fn base_key_prefers_original_uri() {
    let mut ctx = test_context("/rewritten/path");
    ctx.original_uri = Some("/canonical/path".parse().unwrap());
    let request = ctx.req.as_ref().unwrap();
    let key = build_base_key(
        ctx.encrypted,
        request.headers(),
        ctx.original_uri.as_ref(),
        request.uri(),
    );
    assert_eq!(key, "https://example.com/canonical/path");
}
