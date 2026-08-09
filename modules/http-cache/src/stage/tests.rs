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
    let key = build_private_cache_key(&cookies, Some("user"), &[]).unwrap();
    assert!(key.contains("auth=user"));
    assert!(key.contains("cookie:PHPSESSID=1234567890abcdef"));
    assert!(!key.contains("ip="));
}

#[test]
fn client_conditionals_if_none_match() {
    use crate::policy::CacheScope;
    use crate::store::LookupEntry;
    use http::{HeaderValue, Method};
    use std::time::Duration;

    fn entry() -> LookupEntry {
        LookupEntry {
            scope: CacheScope::Public,
            status: http::StatusCode::OK,
            headers: HeaderMap::new(),
            body: None,
            lsc_cookies: Vec::new(),
            age: Duration::from_secs(0),
            etag: Some(HeaderValue::from_static("\"v1\"")),
            last_modified: None,
            stale_if_error: None,
            must_revalidate: false,
            ttl: Duration::from_secs(60),
        }
    }

    let mut headers = HeaderMap::new();
    headers.insert(header::IF_NONE_MATCH, HeaderValue::from_static("\"v1\""));
    assert!(client_conditionals_indicate_not_modified(
        &Method::GET,
        &headers,
        entry().etag.as_ref(),
        entry().last_modified.as_ref(),
    ));

    let mut headers = HeaderMap::new();
    headers.insert(
        header::IF_NONE_MATCH,
        HeaderValue::from_static("\"v2\", \"v1\""),
    );
    assert!(client_conditionals_indicate_not_modified(
        &Method::GET,
        &headers,
        entry().etag.as_ref(),
        entry().last_modified.as_ref(),
    ));

    let mut headers = HeaderMap::new();
    headers.insert(header::IF_NONE_MATCH, HeaderValue::from_static("*"));
    assert!(client_conditionals_indicate_not_modified(
        &Method::GET,
        &headers,
        entry().etag.as_ref(),
        entry().last_modified.as_ref(),
    ));

    let mut headers = HeaderMap::new();
    headers.insert(header::IF_NONE_MATCH, HeaderValue::from_static("W/\"v1\""));
    assert!(client_conditionals_indicate_not_modified(
        &Method::GET,
        &headers,
        entry().etag.as_ref(),
        entry().last_modified.as_ref(),
    ));

    let mut headers = HeaderMap::new();
    headers.insert(header::IF_NONE_MATCH, HeaderValue::from_static("\"v2\""));
    assert!(!client_conditionals_indicate_not_modified(
        &Method::GET,
        &headers,
        entry().etag.as_ref(),
        entry().last_modified.as_ref(),
    ));

    let mut headers = HeaderMap::new();
    headers.insert(header::IF_NONE_MATCH, HeaderValue::from_static("\"v9\""));
    headers.insert(
        header::IF_MODIFIED_SINCE,
        HeaderValue::from_static("Thu, 22 Oct 2015 07:28:00 GMT"),
    );
    // If-None-Match takes precedence: no match means no 304, even though
    // If-Modified-Since alone would have matched.
    let mut matching = entry();
    matching.last_modified = Some(HeaderValue::from_static("Wed, 21 Oct 2015 07:28:00 GMT"));
    assert!(!client_conditionals_indicate_not_modified(
        &Method::GET,
        &headers,
        matching.etag.as_ref(),
        matching.last_modified.as_ref(),
    ));
}

#[test]
fn client_conditionals_if_modified_since() {
    use http::{HeaderValue, Method};
    use std::time::Duration;

    use crate::policy::CacheScope;
    use crate::store::LookupEntry;

    fn entry() -> LookupEntry {
        LookupEntry {
            scope: CacheScope::Public,
            status: http::StatusCode::OK,
            headers: HeaderMap::new(),
            body: None,
            lsc_cookies: Vec::new(),
            age: Duration::from_secs(0),
            etag: None,
            last_modified: Some(HeaderValue::from_static("Wed, 21 Oct 2015 07:28:00 GMT")),
            stale_if_error: None,
            must_revalidate: false,
            ttl: Duration::from_secs(60),
        }
    }

    let mut headers = HeaderMap::new();
    headers.insert(
        header::IF_MODIFIED_SINCE,
        HeaderValue::from_static("Thu, 22 Oct 2015 07:28:00 GMT"),
    );
    assert!(client_conditionals_indicate_not_modified(
        &Method::GET,
        &headers,
        entry().etag.as_ref(),
        entry().last_modified.as_ref(),
    ));

    let mut headers = HeaderMap::new();
    headers.insert(
        header::IF_MODIFIED_SINCE,
        HeaderValue::from_static("Tue, 20 Oct 2015 07:28:00 GMT"),
    );
    assert!(!client_conditionals_indicate_not_modified(
        &Method::GET,
        &headers,
        entry().etag.as_ref(),
        entry().last_modified.as_ref(),
    ));

    // If-Modified-Since applies only to GET and HEAD.
    let headers = HeaderMap::new();
    assert!(!client_conditionals_indicate_not_modified(
        &Method::POST,
        &headers,
        entry().etag.as_ref(),
        entry().last_modified.as_ref(),
    ));

    // No conditional headers: full representation.
    let headers = HeaderMap::new();
    assert!(!client_conditionals_indicate_not_modified(
        &Method::GET,
        &headers,
        entry().etag.as_ref(),
        entry().last_modified.as_ref(),
    ));
}

#[test]
fn report_emits_exactly_one_request_metric_per_outcome() {
    use std::sync::Mutex;

    use ferron_observability::{Event, EventSink, MetricAttributeValue, MetricEvent};

    #[derive(Default)]
    struct CapturingSink(Mutex<Vec<Event>>);
    impl EventSink for CapturingSink {
        fn emit(&self, event: Event) {
            self.0.lock().unwrap().push(event);
        }
    }

    let zone = CacheZoneId::Host("example.com".to_string());

    for result in ["hit", "stale", "miss", "revalidated"] {
        let sink = Arc::new(CapturingSink::default());
        let mut ctx = test_context("/metric");
        ctx.events.add_sink(sink.clone());

        report(
            &mut ctx,
            CacheOutcome {
                result,
                zone_id: &zone,
                key: "https://example.com/metric",
                scope: None,
                items: Some(7),
                stored: None,
                evictions: None,
                detail: None,
                key_uri: None,
                key_method: None,
                bypass_reason: None,
                evaluated_cookies: None,
                coalesced_wait_ms: None,
                mark_uncoalesced: true,
                metric_result: None,
            },
        );

        let events = sink.0.lock().unwrap();
        let request_metrics: Vec<&MetricEvent> = events
            .iter()
            .filter_map(|event| match event {
                Event::Metric(metric) if metric.name == "ferron.cache.requests" => Some(metric),
                _ => None,
            })
            .collect();
        assert_eq!(
            request_metrics.len(),
            1,
            "report must emit exactly one request metric for result `{result}`"
        );
        let is_labeled = request_metrics[0].attributes.iter().any(|(key, value)| {
            *key == "ferron.cache.result"
                && matches!(value, MetricAttributeValue::StaticStr(label) if *label == result)
        });
        assert!(
            is_labeled,
            "request metric for `{result}` must carry the matching result label"
        );
    }
}

#[test]
fn private_key_requires_identity_without_falling_back_to_ip() {
    // No auth, no private cookie, no declared vary cookie: no identity, so no
    // key. The caller must treat the response as not storable in private scope
    // rather than keying on the client IP alone (F12).
    let empty = ahash::AHashMap::default();
    assert!(build_private_cache_key(&empty, None, &[]).is_none());

    let mut cookies = ahash::AHashMap::default();
    cookies.insert("visitor_id".to_string(), "abcdef1234567890".to_string());
    assert!(build_private_cache_key(&cookies, None, &[]).is_none());

    // A declared vary cookie is an identifying component.
    let key = build_private_cache_key(&cookies, None, &["visitor_id".to_string()]);
    assert_eq!(key.as_deref(), Some("cookie:visitor_id=abcdef1234567890"));
}

#[test]
fn private_key_caps_cookie_components_and_value_length() {
    let mut cookies = ahash::AHashMap::default();
    for index in 0..20 {
        cookies.insert(
            format!("session_{index}"),
            format!("cookie_value_for_user_{index}_abcdefgh"),
        );
    }
    // With a declared vary cookie each one is an identity candidate, but the
    // key must cap the number of cookie components (F13).
    let vary_names: Vec<String> = (0..20).map(|i| format!("session_{i}")).collect();
    let key = build_private_cache_key(&cookies, None, &vary_names).unwrap();
    assert_eq!(key.matches("cookie:").count(), 8);

    // Individual cookie values longer than the cap are truncated.
    let mut long = ahash::AHashMap::default();
    let long_value = "x".repeat(1000);
    long.insert("lsc_private".to_string(), long_value.clone());
    let key = build_private_cache_key(&long, None, &[]).unwrap();
    assert!(key.contains("cookie:lsc_private="));
    assert!(key.split('=').next_back().unwrap().len() <= 256);
}

#[test]
fn private_key_ignores_arbitrary_cookies_without_vary_declaration() {
    // Arbitrary cookies must not appear in the key unless declared as vary or
    // private cookie names (F13).
    let mut cookies = ahash::AHashMap::default();
    cookies.insert("visitor_id".to_string(), "abcdef1234567890".to_string());
    cookies.insert("tracking".to_string(), "uuid_value_16chars".to_string());
    let key = build_private_cache_key(&cookies, None, &["tracking".to_string()]).unwrap();
    assert!(key.contains("cookie:tracking="));
    assert!(!key.contains("visitor_id"));
}

#[test]
fn base_key_uses_scheme_host_and_path() {
    let ctx = test_context("/test?q=1");
    let request = ctx.req.as_ref().unwrap();
    let key = build_base_key(
        ctx.encrypted,
        request.headers(),
        None,
        request.uri(),
        ctx.hostname.as_deref(),
    );
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
        ctx.hostname.as_deref(),
    );
    assert_eq!(key, "https://example.com/canonical/path");
}

#[test]
fn base_key_prefers_resolved_host_over_host_header() {
    let ctx = test_context("/page");

    // A spoofed Host header must not change the key.
    let mut headers = http::HeaderMap::new();
    headers.insert(http::header::HOST, "attacker.example".parse().unwrap());
    let key = build_base_key(
        ctx.encrypted,
        &headers,
        None,
        ctx.req.as_ref().unwrap().uri(),
        ctx.hostname.as_deref(),
    );
    assert_eq!(key, "https://example.com/page");
}

#[test]
fn base_key_normalizes_host_case() {
    let ctx = test_context("/page");

    // Resolved host with mixed case is lowercased.
    let key = build_base_key(
        ctx.encrypted,
        &http::HeaderMap::new(),
        None,
        ctx.req.as_ref().unwrap().uri(),
        Some("EXAMPLE.COM"),
    );
    assert_eq!(key, "https://example.com/page");

    // Host header differing in case from the vhost still hits the same key.
    let mut headers = http::HeaderMap::new();
    headers.insert(http::header::HOST, "Example.COM".parse().unwrap());
    let key = build_base_key(
        ctx.encrypted,
        &headers,
        None,
        ctx.req.as_ref().unwrap().uri(),
        None,
    );
    assert_eq!(key, "https://example.com/page");
}

#[test]
fn base_key_falls_back_to_host_header_without_resolved_host() {
    let mut ctx = test_context("/page");
    ctx.hostname = None;
    let request = ctx.req.as_ref().unwrap();
    let key = build_base_key(
        ctx.encrypted,
        request.headers(),
        None,
        request.uri(),
        ctx.hostname.as_deref(),
    );
    assert_eq!(key, "https://example.com/page");
}

#[test]
fn cache_key_fingerprint_does_not_leak_query_string() {
    use super::key::cache_key_fingerprint;

    // A short key with a secret query value must drop the query entirely.
    let key = "https://example.com/search?token=super-secret";
    let fingerprint = cache_key_fingerprint(key);
    assert_eq!(fingerprint, "https://example.com/search");

    // A long URL: the query is stripped before truncation, so no part of the
    // secret query survives even in the truncated prefix.
    let key = format!(
        "https://example.com/very/long/path/{}/search?token=super-secret&page={}",
        "x".repeat(80),
        123456789,
    );
    let fingerprint = cache_key_fingerprint(&key);
    assert!(
        !fingerprint.contains('?'),
        "fingerprint must not contain a query string, got: {fingerprint}"
    );
    assert!(
        !fingerprint.contains("secret"),
        "fingerprint must not leak the query value, got: {fingerprint}"
    );

    // The scope/vary tail after the base URL is preserved, so variants stay
    // distinguishable in logs.
    let key = "https://example.com/p\nscope=public";
    let fingerprint = cache_key_fingerprint(key);
    assert_eq!(fingerprint, "https://example.com/p\nscope=public");
    assert!(fingerprint.contains("scope=public"));
}

#[test]
fn propagation_paths_map_selectors_and_deduplicate() {
    use crate::lscache::{PurgeOperation, PurgeSelector};
    use crate::policy::CacheScope;

    let operations = vec![PurgeOperation {
        scope: CacheScope::Public,
        selectors: vec![
            PurgeSelector::All,
            PurgeSelector::UrlPath("/a".to_string()),
            PurgeSelector::Tag("v1".to_string()),
            PurgeSelector::Url("/b?x=1".to_string()),
            PurgeSelector::UrlPath("/a".to_string()),
        ],
        stale: false,
    }];
    assert_eq!(
        super::purge::collect_propagation_paths(&operations),
        vec!["*", "/a", "tag=v1", "/b?x=1"]
    );
}

#[test]
fn vary_rule_ignores_conditional_and_range_headers() {
    use super::key::build_vary_rule;

    let mut response_headers = http::HeaderMap::new();
    response_headers.insert(
        http::header::VARY,
        http::HeaderValue::from_static(
            "Accept-Encoding, If-Match, If-Modified-Since, If-None-Match, If-Range, If-Unmodified-Since, Range",
        ),
    );

    let config = crate::config::CacheConfig::default();
    let rule = build_vary_rule(
        &response_headers,
        &config,
        &crate::lscache::LiteSpeedVary::default(),
    )
    .unwrap()
    .expect("a vary rule should be built");

    assert_eq!(rule.header_names.len(), 1, "only Accept-Encoding survives");
    assert_eq!(rule.header_names[0].as_str(), "accept-encoding");
    assert!(rule.cookie_names.is_empty());
}

#[test]
fn purge_reports_purged_and_remaining_entry_counts() {
    use std::time::Duration;

    use bytes::Bytes;
    use http::StatusCode;

    use crate::lscache::{PurgeOperation, PurgeSelector};
    use crate::policy::CacheScope;
    use crate::store::{CacheStore, StoredEntry};

    let mut ctx = test_context("/purge/me");
    let store = CacheStore::new(100);

    let entry = StoredEntry {
        scope: CacheScope::Public,
        base_key: "https://example.com/keep".to_string(),
        vary: crate::store::VaryRule {
            header_names: Vec::new(),
            cookie_names: Vec::new(),
            value: None,
        },
        status: StatusCode::OK,
        headers: http::HeaderMap::new(),
        body: Some(Bytes::from_static(b"hello")),
        lsc_cookies: Vec::new(),
        created_at: std::time::Instant::now(),
        ttl: Duration::from_secs(60),
        access_at: 0,
        private_key: None,
        tags: Vec::new(),
        purge_url: "/keep".to_string(),
        purge_host: "example.com".to_string(),
        etag: None,
        last_modified: None,
        stale_while_revalidate: Some(Duration::from_secs(10)),
        stale_if_error: None,
        must_revalidate: false,
    };
    store.insert_with_request(
        entry.clone(),
        None,
        &http::HeaderMap::new(),
        &Default::default(),
    );

    let second = StoredEntry {
        base_key: "https://example.com/purge/me".to_string(),
        purge_url: "/purge/me".to_string(),
        ..entry
    };
    store.insert_with_request(second, None, &http::HeaderMap::new(), &Default::default());

    let operations = vec![PurgeOperation {
        scope: CacheScope::Public,
        selectors: vec![PurgeSelector::UrlPath("/purge/me".to_string())],
        stale: false,
    }];
    let stats = super::purge::purge(
        &mut ctx,
        &CacheZoneId::Host("example.com".to_string()),
        &store,
        &operations,
        None,
        Some("example.com"),
        false,
        &crate::config::PurgePropagationConfig::default(),
    );

    assert_eq!(stats.purged, 1);
    assert_eq!(store.len(), 1);
}

#[test]
fn purge_allowed_rejects_unauthenticated_without_allowlist() {
    let ip: std::net::IpAddr = "192.0.2.10".parse().unwrap();
    assert!(!purge_allowed(ip, &[], false, None));
    assert!(!purge_allowed(ip, &[], true, None));
}

#[test]
fn purge_allowed_rejects_foreign_authenticated_user() {
    // An authenticated user alone must not authorize a purge: the request's
    // basic_auth block must be in scope.
    let ip: std::net::IpAddr = "192.0.2.10".parse().unwrap();
    assert!(!purge_allowed(ip, &[], false, Some("alice")));
}

#[test]
fn purge_allowed_accepts_in_scope_authenticated_user() {
    let ip: std::net::IpAddr = "192.0.2.10".parse().unwrap();
    assert!(purge_allowed(ip, &[], true, Some("alice")));
}

#[test]
fn purge_allowed_accepts_allowlisted_ip_without_auth() {
    let ip: std::net::IpAddr = "192.0.2.10".parse().unwrap();
    let allowlist = vec!["192.0.2.0/24".parse::<cidr::IpCidr>().unwrap()];
    assert!(purge_allowed(ip, &allowlist, false, None));
}

#[test]
fn purge_allowed_rejects_ip_outside_allowlist() {
    let ip: std::net::IpAddr = "198.51.100.5".parse().unwrap();
    let allowlist = vec!["192.0.2.0/24".parse::<cidr::IpCidr>().unwrap()];
    assert!(!purge_allowed(ip, &allowlist, false, None));
}

#[test]
fn propagation_secret_must_match_configured_secret() {
    use http::header::HeaderValue;
    use http::HeaderMap;

    let mut headers = HeaderMap::new();

    // Missing secret is rejected even when one is configured.
    assert!(!propagation_secret_verified(&headers, Some("hunter2")));

    // Wrong secret is rejected.
    headers.insert(
        super::purge::PURGE_SECRET_HEADER,
        HeaderValue::from_static("wrong"),
    );
    assert!(!propagation_secret_verified(&headers, Some("hunter2")));

    // Matching secret is accepted.
    headers.insert(
        super::purge::PURGE_SECRET_HEADER,
        HeaderValue::from_static("hunter2"),
    );
    assert!(propagation_secret_verified(&headers, Some("hunter2")));

    // A propagation claim with no configured secret is never accepted.
    assert!(!propagation_secret_verified(&headers, None));
}

#[test]
fn get_or_build_retries_failed_builds() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::OnceLock;

    let attempts = AtomicUsize::new(0);
    let cell: OnceLock<i32> = OnceLock::new();
    let build = || {
        let attempt = attempts.fetch_add(1, Ordering::SeqCst);
        if attempt == 0 {
            Err("first build fails")
        } else {
            Ok(42)
        }
    };

    // A failed build is not cached; the next call retries and succeeds.
    assert!(super::purge::get_or_build(&cell, build).is_err());
    assert_eq!(super::purge::get_or_build(&cell, build), Ok(&42));

    // Once built, the value is cached and the builder does not run again.
    assert_eq!(super::purge::get_or_build(&cell, build), Ok(&42));
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
}

#[test]
fn coalesce_timeout_defaults_and_parses() {
    use ferron_core::config::layer::LayeredConfiguration;
    use ferron_core::config::{
        ServerConfigurationBlockBuilder, ServerConfigurationDirectiveEntry,
        ServerConfigurationValue,
    };

    fn layered_config(coalesce_timeout: Option<u64>) -> LayeredConfiguration {
        let mut builder = ServerConfigurationBlockBuilder::new();
        if let Some(secs) = coalesce_timeout {
            builder = builder.directive(
                "coalesce_timeout",
                ServerConfigurationDirectiveEntry {
                    args: vec![ServerConfigurationValue::Number(secs as i64, None)],
                    children: None,
                    span: None,
                },
            );
        }
        let block = ServerConfigurationBlockBuilder::new()
            .directive_with_block("cache", Vec::<String>::new(), builder.build())
            .build();
        let mut layered = LayeredConfiguration::new();
        layered.add_layer(std::sync::Arc::new(block));
        layered
    }

    let config = super::parse_cache_config(&layered_config(None));
    assert_eq!(
        config.coalesce_timeout,
        std::time::Duration::from_secs(5),
        "coalesce_timeout must default to 5 seconds"
    );

    let config = super::parse_cache_config(&layered_config(Some(2)));
    assert_eq!(
        config.coalesce_timeout,
        std::time::Duration::from_secs(2),
        "coalesce_timeout must parse integer seconds"
    );
}

#[test]
fn config_cache_is_keyed_per_host_and_cleared_on_reload() {
    use ferron_core::config::layer::LayeredConfiguration;
    use ferron_core::config::ServerConfigurationBlockBuilder;

    fn layered_config(max_response_size: u64) -> LayeredConfiguration {
        use ferron_core::config::{ServerConfigurationDirectiveEntry, ServerConfigurationValue};
        let entry = ServerConfigurationDirectiveEntry {
            args: vec![ServerConfigurationValue::Number(
                max_response_size as i64,
                None,
            )],
            children: None,
            span: None,
        };
        let block = ServerConfigurationBlockBuilder::new()
            .directive_with_block(
                "cache",
                Vec::<String>::new(),
                ServerConfigurationBlockBuilder::new()
                    .directive("max_response_size", entry)
                    .build(),
            )
            .build();
        let mut layered = LayeredConfiguration::new();
        layered.add_layer(std::sync::Arc::new(block));
        layered
    }

    let stage = HttpCacheStage::new();
    let mut ctx_a = test_context("/a");
    ctx_a.hostname = Some("a.example.com".to_string());
    ctx_a.configuration = layered_config(1024);
    let mut ctx_b = test_context("/b");
    ctx_b.hostname = Some("b.example.com".to_string());
    ctx_b.configuration = layered_config(2048);

    let config_a = stage.get_config(&ctx_a);
    assert_eq!(config_a.max_response_size, 1024);
    assert!(config_a.enabled);
    assert_eq!(stage.configs.len(), 1);

    let config_b = stage.get_config(&ctx_b);
    assert_eq!(config_b.max_response_size, 2048);
    assert_eq!(stage.configs.len(), 2);

    assert_eq!(stage.get_config(&ctx_a).max_response_size, 1024);
    assert_eq!(stage.configs.len(), 2);

    ferron_core::admin::ADMIN_METRICS
        .reload_metrics
        .write()
        .active_generation += 1;

    assert_eq!(stage.get_config(&ctx_b).max_response_size, 2048);
    assert_eq!(stage.configs.len(), 1);
}
