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
