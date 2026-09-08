use super::*;
use bytes::Bytes;
use ferron_core::config::layer::LayeredConfiguration;
use ferron_core::config::{
    ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue,
};
use ferron_http::HttpRequest;
use ferron_observability::CompositeEventSink;
use http::Request;
use http_body_util::{BodyExt, Empty};
use rustc_hash::FxHashMap;

fn make_test_context(remote_address: &str, config: Option<LayeredConfiguration>) -> HttpContext {
    let req: HttpRequest = Request::builder()
        .uri("/path")
        .body(Empty::<Bytes>::new().map_err(|e| match e {}).boxed_unsync())
        .unwrap();

    let mut ctx = HttpContext::default();
    ctx.req = Some(req);
    ctx.events = CompositeEventSink::new(Vec::new());
    ctx.configuration = config.unwrap_or_default();
    ctx.encrypted = false;
    ctx.local_address = Some("0.0.0.0:80".parse().unwrap());
    ctx.remote_address = Some(remote_address.parse().unwrap());
    ctx
}

fn make_rate_limit_config(rate: u64, burst: u64) -> LayeredConfiguration {
    make_config_with_backend(rate, burst, false, None)
}

fn make_backend_block(
    backend_type: &str,
    url: Option<&str>,
    fail_open: Option<bool>,
    key_prefix: Option<String>,
) -> ServerConfigurationBlock {
    let mut inner: FxHashMap<String, Vec<ServerConfigurationDirectiveEntry>> = FxHashMap::default();
    inner.insert(
        "type".to_string(),
        vec![ServerConfigurationDirectiveEntry {
            args: vec![ServerConfigurationValue::String(backend_type.into(), None)],
            children: None,
            span: None,
        }],
    );
    if let Some(url) = url {
        inner.insert(
            "url".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String(url.into(), None)],
                children: None,
                span: None,
            }],
        );
    }
    if let Some(fail_open) = fail_open {
        inner.insert(
            "fail_open".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::Boolean(fail_open, None)],
                children: None,
                span: None,
            }],
        );
    }
    if let Some(prefix) = key_prefix {
        inner.insert(
            "key_prefix".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String(prefix, None)],
                children: None,
                span: None,
            }],
        );
    }
    ServerConfigurationBlock {
        directives: Arc::new(inner),
        matchers: FxHashMap::default(),
        span: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn make_config_with_backend(
    rate: u64,
    burst: u64,
    throttle: bool,
    backend: Option<ServerConfigurationBlock>,
) -> LayeredConfiguration {
    let mut inner_directives: FxHashMap<String, Vec<ServerConfigurationDirectiveEntry>> =
        FxHashMap::default();
    inner_directives.insert(
        "rate".to_string(),
        vec![ServerConfigurationDirectiveEntry {
            args: vec![ServerConfigurationValue::Number(rate as i64, None)],
            children: None,
            span: None,
        }],
    );
    inner_directives.insert(
        "burst".to_string(),
        vec![ServerConfigurationDirectiveEntry {
            args: vec![ServerConfigurationValue::Number(burst as i64, None)],
            children: None,
            span: None,
        }],
    );
    if throttle {
        inner_directives.insert(
            "throttle".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![],
                children: None,
                span: None,
            }],
        );
    }

    let mut directives: FxHashMap<String, Vec<ServerConfigurationDirectiveEntry>> =
        FxHashMap::default();
    directives.insert(
        "rate_limit".to_string(),
        vec![ServerConfigurationDirectiveEntry {
            args: vec![],
            children: Some(ServerConfigurationBlock {
                directives: Arc::new(inner_directives),
                matchers: FxHashMap::default(),
                span: None,
            }),
            span: None,
        }],
    );
    if let Some(backend_block) = backend {
        directives.insert(
            "rate_limit_backend".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![],
                children: Some(backend_block),
                span: None,
            }],
        );
    }

    let mut config = LayeredConfiguration::new();
    config.add_layer(Arc::new(ServerConfigurationBlock {
        directives: Arc::new(directives),
        matchers: FxHashMap::default(),
        span: None,
    }));
    config
}

fn unique_prefix(tag: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("test:rl:{tag}:{}-{nanos}:", std::process::id())
}

fn redis_available() -> bool {
    // Hmm, unit tests turned into integration/E2E tests involving connecting
    // with an actual Redis/Valkey server... :')
    use std::net::TcpStream;
    TcpStream::connect_timeout(
        &"127.0.0.1:6379".parse().unwrap(),
        std::time::Duration::from_millis(300),
    )
    .is_ok()
}

#[tokio::test]
async fn allows_requests_within_limit() {
    let engine = Arc::new(RateLimitEngine::new());
    let stage = RateLimitStage::new(engine);
    let config = make_rate_limit_config(10, 5);

    for i in 0..15 {
        let mut ctx = make_test_context(&format!("192.0.2.1:{}", 20000 + i), Some(config.clone()));
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result, "request should be allowed");
        assert!(ctx.res.is_none());
    }
}

#[tokio::test]
async fn rejects_when_bucket_exhausted() {
    let engine = Arc::new(RateLimitEngine::new());
    let stage = RateLimitStage::new(engine);
    let config = make_rate_limit_config(5, 0);

    // First 5 requests should pass
    for i in 0..5 {
        let mut ctx = make_test_context(&format!("192.0.2.1:1234{}", i), Some(config.clone()));
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result);
        assert!(ctx.res.is_none());
    }

    // 6th should be rejected
    let mut ctx = make_test_context("192.0.2.1:12345", Some(config));
    let result = stage.run(&mut ctx).await.unwrap();
    assert!(!result, "should stop pipeline when rate limited");
    assert!(ctx.res.is_some());
}

#[tokio::test]
async fn different_ips_get_separate_buckets() {
    let engine = Arc::new(RateLimitEngine::new());
    let stage = RateLimitStage::new(engine);
    let config = make_rate_limit_config(1, 0);

    // IP1 uses its token
    let mut ctx1 = make_test_context("192.0.2.1:12345", Some(config.clone()));
    assert!(stage.run(&mut ctx1).await.unwrap());

    // IP2 should still have its own token
    let mut ctx2 = make_test_context("192.0.2.2:12345", Some(config.clone()));
    assert!(stage.run(&mut ctx2).await.unwrap());

    // IP1 should be exhausted
    let mut ctx1 = make_test_context("192.0.2.1:12345", Some(config));
    assert!(!stage.run(&mut ctx1).await.unwrap());
}

#[tokio::test]
async fn sets_retry_after_header() {
    let engine = Arc::new(RateLimitEngine::new());
    let stage = RateLimitStage::new(engine);
    let config = make_rate_limit_config(1, 0);

    // Use the token
    let mut ctx1 = make_test_context("192.0.2.1:12345", Some(config.clone()));
    stage.run(&mut ctx1).await.unwrap();

    // Next request should be rejected with Retry-After
    let mut ctx2 = make_test_context("192.0.2.1:12345", Some(config));
    stage.run(&mut ctx2).await.unwrap();

    if let Some(HttpResponse::BuiltinError(status, headers)) = ctx2.res {
        assert!(headers.unwrap().contains_key(http::header::RETRY_AFTER));
        assert_eq!(status, 429);
    } else {
        panic!("Expected rate limit response");
    }
}

#[tokio::test]
async fn redis_backend_shares_limit_across_engines() {
    if !redis_available() {
        eprintln!("skipping redis stage test: localhost:6379 unreachable");
        return;
    }
    // Two engines with the same Redis URL simulate two Ferron nodes.
    // rate 1 + burst 1 = capacity 2, refill 1/sec (robust against
    // timing flakes; 1s between refills).
    let prefix = unique_prefix("shared");
    let backend = make_backend_block(
        "redis",
        Some("redis://127.0.0.1:6379/0"),
        None,
        Some(prefix),
    );
    let config = make_config_with_backend(1, 1, false, Some(backend));

    let engine_a = Arc::new(RateLimitEngine::new());
    let engine_b = Arc::new(RateLimitEngine::new());
    let stage_a = RateLimitStage::new(engine_a);
    let stage_b = RateLimitStage::new(engine_b);

    let mut ctx = make_test_context("192.0.2.99:12345", Some(config.clone()));
    assert!(stage_a.run(&mut ctx).await.unwrap());
    let mut ctx = make_test_context("192.0.2.99:12345", Some(config.clone()));
    assert!(stage_a.run(&mut ctx).await.unwrap());
    // Third request on the *other* engine must still be denied (shared).
    let mut ctx = make_test_context("192.0.2.99:12345", Some(config));
    assert!(!stage_b.run(&mut ctx).await.unwrap());
    assert!(ctx.res.is_some());
}

#[tokio::test]
async fn redis_fail_open_allows_when_unreachable() {
    let backend = make_backend_block(
        "redis",
        Some("redis://127.0.0.1:6399/0"),
        Some(true),
        Some(unique_prefix("open")),
    );
    let config = make_config_with_backend(1, 0, false, Some(backend));
    let stage = RateLimitStage::new(Arc::new(RateLimitEngine::new()));

    for _ in 0..3 {
        let mut ctx = make_test_context("192.0.2.50:12345", Some(config.clone()));
        let result = stage.run(&mut ctx).await.unwrap();
        assert!(result, "fail-open should allow on backend error");
        assert!(ctx.res.is_none());
    }
}

#[tokio::test]
async fn redis_fail_closed_denies_when_unreachable() {
    let backend = make_backend_block(
        "redis",
        Some("redis://127.0.0.1:6399/0"),
        Some(false),
        Some(unique_prefix("closed")),
    );
    let config = make_config_with_backend(100, 100, false, Some(backend));
    let stage = RateLimitStage::new(Arc::new(RateLimitEngine::new()));

    let mut ctx = make_test_context("192.0.2.51:12345", Some(config));
    let result = stage.run(&mut ctx).await.unwrap();
    assert!(!result, "fail-closed should deny on backend error");
    assert!(ctx.res.is_some());
}

#[tokio::test]
async fn redis_throttle_sleeps_once_and_allows() {
    if !redis_available() {
        eprintln!("skipping redis stage test: localhost:6379 unreachable");
        return;
    }
    let backend = make_backend_block(
        "redis",
        Some("redis://127.0.0.1:6379/0"),
        None,
        Some(unique_prefix("throttle")),
    );
    // rate 5/sec, no burst, throttle => 6th request waits ~200ms.
    let config = make_config_with_backend(5, 0, true, Some(backend));
    let stage = RateLimitStage::new(Arc::new(RateLimitEngine::new()));

    for _ in 0..5 {
        let mut ctx = make_test_context("192.0.2.60:12345", Some(config.clone()));
        assert!(stage.run(&mut ctx).await.unwrap());
    }

    let start = std::time::Instant::now();
    let mut ctx = make_test_context("192.0.2.60:12345", Some(config));
    let result = stage.run(&mut ctx).await.unwrap();
    assert!(result, "throttle should allow after a single sleep");
    assert!(ctx.res.is_none());
    assert!(
        start.elapsed() >= std::time::Duration::from_millis(50),
        "expected throttle sleep, elapsed {:?}",
        start.elapsed()
    );
}
