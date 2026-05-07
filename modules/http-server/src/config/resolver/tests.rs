use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashMap},
    net::IpAddr,
    sync::Arc,
};

use ferron_core::config::{
    layer::LayeredConfiguration, ServerConfigurationBlock, ServerConfigurationDirectiveEntry,
    ServerConfigurationMatcherExpr, ServerConfigurationMatcherOperand,
    ServerConfigurationMatcherOperator, ServerConfigurationValue,
};
use ferron_http::{HttpContext, HttpRequest};
use ferron_observability::CompositeEventSink;
use http_body_util::{BodyExt, Empty};
use rustc_hash::FxHashMap;
use typemap_rev::TypeMap;

use super::super::prepare::{
    PreparedConfiguration, PreparedHostConfigurationBlock, PreparedHostConfigurationErrorConfig,
    PreparedHostConfigurationMatch, PreparedHostConfigurationMatcher,
};
use super::resolver::*;
use crate::config::prepare::HostConfigs;

fn make_test_context(req: HttpRequest, hostname: &str) -> HttpContext {
    HttpContext {
        req: Some(req),
        res: None,
        events: CompositeEventSink::new(Vec::new()),
        configuration: LayeredConfiguration::default(),
        hostname: Some(hostname.to_string()),
        variables: FxHashMap::default(),
        previous_error: None,
        original_uri: None,
        routing_uri: None,
        encrypted: false,
        local_address: "127.0.0.1:80".parse().unwrap(),
        remote_address: "127.0.0.1:12345".parse().unwrap(),
        auth_user: None,
        https_port: None,
        extensions: TypeMap::new(),
    }
}

fn empty_request() -> HttpRequest {
    http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync())
}

fn string_block(name: &str, value: &str) -> PreparedHostConfigurationBlock {
    let mut directives = HashMap::new();
    directives.insert(
        name.to_string(),
        vec![ServerConfigurationDirectiveEntry {
            args: vec![ServerConfigurationValue::String(value.to_string(), None)],
            children: None,
            span: None,
        }],
    );

    PreparedHostConfigurationBlock {
        directives: Arc::new(directives),
        matches: Vec::new(),
        error_config: Vec::new(),
    }
}

fn read_string(config: &LayeredConfiguration, name: &str) -> Option<String> {
    config
        .get_value(name, true)
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

#[test]
fn layers_generic_ip_and_exact_host_blocks_by_specificity() {
    let mut generic_hosts = HostConfigs::new();
    generic_hosts.insert(None, Arc::new(string_block("generic_default", "yes")));
    generic_hosts.insert(
        Some("example.com".to_string()),
        Arc::new(string_block("generic_host", "yes")),
    );

    let mut scoped_hosts = HostConfigs::new();
    scoped_hosts.insert(None, Arc::new(string_block("ip_default", "yes")));
    scoped_hosts.insert(
        Some("example.com".to_string()),
        Arc::new(string_block("ip_host", "yes")),
    );

    let mut prepared = PreparedConfiguration::new();
    prepared.insert(None, generic_hosts);
    prepared.insert(Some("127.0.0.1".parse().unwrap()), scoped_hosts);

    let resolver = ThreeStageResolver::from_prepared(prepared);
    let result = resolver
        .resolve(
            "127.0.0.1".parse().unwrap(),
            "example.com",
            "/",
            &make_test_context(empty_request(), "example.com"),
        )
        .expect("request should resolve");

    assert_eq!(
        read_string(&result.configuration, "generic_default").as_deref(),
        Some("yes")
    );
    assert_eq!(
        read_string(&result.configuration, "ip_default").as_deref(),
        Some("yes")
    );
    assert_eq!(
        read_string(&result.configuration, "generic_host").as_deref(),
        Some("yes")
    );
    assert_eq!(
        read_string(&result.configuration, "ip_host").as_deref(),
        Some("yes")
    );
    assert_eq!(
        result.location_path.hostname_segments,
        vec!["example".to_string(), "com".to_string()]
    );
}

#[test]
fn resolves_wildcard_hosts_using_lookup_tree_keys() {
    let mut hosts = HostConfigs::new();
    hosts.insert(
        Some("*.example.com".to_string()),
        Arc::new(string_block("host", "wildcard")),
    );
    hosts.insert(None, Arc::new(string_block("host", "default")));

    let mut prepared = PreparedConfiguration::new();
    prepared.insert(Some("127.0.0.1".parse().unwrap()), hosts);

    let resolver = ThreeStageResolver::from_prepared(prepared);
    let wildcard = resolver
        .resolve(
            "127.0.0.1".parse().unwrap(),
            "deep.api.example.com",
            "/",
            &make_test_context(empty_request(), "deep.api.example.com"),
        )
        .expect("wildcard host should resolve");

    assert_eq!(
        read_string(&wildcard.configuration, "host").as_deref(),
        Some("wildcard")
    );
    assert_eq!(
        wildcard.location_path.hostname_segments,
        vec!["*".to_string(), "example".to_string(), "com".to_string()]
    );
}

#[test]
fn layers_multiple_matching_locations_additively() {
    let host = PreparedHostConfigurationBlock {
        directives: Arc::new(HashMap::new()),
        matches: vec![
            PreparedHostConfigurationMatch {
                matcher: PreparedHostConfigurationMatcher::Location("/".to_string()),
                config: Arc::new(string_block("root_location", "yes")),
            },
            PreparedHostConfigurationMatch {
                matcher: PreparedHostConfigurationMatcher::Location("/api".to_string()),
                config: Arc::new(string_block("api_location", "yes")),
            },
        ],
        error_config: Vec::new(),
    };

    let mut hosts = HostConfigs::new();
    hosts.insert(Some("example.com".to_string()), Arc::new(host));

    let mut prepared = PreparedConfiguration::new();
    prepared.insert(Some("127.0.0.1".parse().unwrap()), hosts);

    let resolver = ThreeStageResolver::from_prepared(prepared);
    let result = resolver
        .resolve(
            "127.0.0.1".parse().unwrap(),
            "example.com",
            "/api/users",
            &make_test_context(empty_request(), "example.com"),
        )
        .expect("request should resolve");

    assert_eq!(
        read_string(&result.configuration, "root_location").as_deref(),
        Some("yes")
    );
    assert_eq!(
        read_string(&result.configuration, "api_location").as_deref(),
        Some("yes")
    );
    assert_eq!(result.location_path.path_segments, vec!["api".to_string()]);
}

#[test]
fn layers_multiple_matching_conditionals_additively() {
    let expr_get = ServerConfigurationMatcherExpr {
        left: ServerConfigurationMatcherOperand::Identifier("request.method".to_string()),
        right: ServerConfigurationMatcherOperand::String("GET".to_string()),
        op: ServerConfigurationMatcherOperator::Eq,
    };
    let expr_root = ServerConfigurationMatcherExpr {
        left: ServerConfigurationMatcherOperand::Identifier("request.uri.path".to_string()),
        right: ServerConfigurationMatcherOperand::String("/".to_string()),
        op: ServerConfigurationMatcherOperator::Eq,
    };

    let block = PreparedHostConfigurationBlock {
        directives: Arc::new(HashMap::new()),
        matches: vec![
            PreparedHostConfigurationMatch {
                matcher: PreparedHostConfigurationMatcher::IfConditional(vec![expr_get]),
                config: Arc::new(string_block("if_get", "yes")),
            },
            PreparedHostConfigurationMatch {
                matcher: PreparedHostConfigurationMatcher::IfConditional(vec![expr_root]),
                config: Arc::new(string_block("if_root", "yes")),
            },
        ],
        error_config: Vec::new(),
    };

    let mut hosts = HostConfigs::new();
    hosts.insert(Some("example.com".to_string()), Arc::new(block));

    let mut prepared = PreparedConfiguration::new();
    prepared.insert(Some("127.0.0.1".parse().unwrap()), hosts);

    let resolver = ThreeStageResolver::from_prepared(prepared);
    let result = resolver
        .resolve(
            "127.0.0.1".parse().unwrap(),
            "example.com",
            "/",
            &make_test_context(empty_request(), "example.com"),
        )
        .expect("request should resolve");

    assert_eq!(
        read_string(&result.configuration, "if_get").as_deref(),
        Some("yes")
    );
    assert_eq!(
        read_string(&result.configuration, "if_root").as_deref(),
        Some("yes")
    );
    assert_eq!(result.location_path.conditionals.len(), 2);
}

#[test]
fn resolves_nested_location_inside_a_conditional_scope() {
    let expr_post = ServerConfigurationMatcherExpr {
        left: ServerConfigurationMatcherOperand::Identifier("request.method".to_string()),
        right: ServerConfigurationMatcherOperand::String("POST".to_string()),
        op: ServerConfigurationMatcherOperator::Eq,
    };

    let conditional_block = PreparedHostConfigurationBlock {
        directives: Arc::new(HashMap::new()),
        matches: vec![PreparedHostConfigurationMatch {
            matcher: PreparedHostConfigurationMatcher::Location("/admin".to_string()),
            config: Arc::new(string_block("nested", "hit")),
        }],
        error_config: Vec::new(),
    };

    let host = PreparedHostConfigurationBlock {
        directives: Arc::new(HashMap::new()),
        matches: vec![PreparedHostConfigurationMatch {
            matcher: PreparedHostConfigurationMatcher::IfConditional(vec![expr_post]),
            config: Arc::new(conditional_block),
        }],
        error_config: Vec::new(),
    };

    let mut hosts = HostConfigs::new();
    hosts.insert(Some("example.com".to_string()), Arc::new(host));

    let mut prepared = PreparedConfiguration::new();
    prepared.insert(Some("127.0.0.1".parse().unwrap()), hosts);

    let resolver = ThreeStageResolver::from_prepared(prepared);
    let mut request = http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync());
    *request.method_mut() = http::Method::POST;

    let result = resolver
        .resolve(
            "127.0.0.1".parse().unwrap(),
            "example.com",
            "/admin/panel",
            &make_test_context(request, "example.com"),
        )
        .expect("request should resolve");

    assert_eq!(
        read_string(&result.configuration, "nested").as_deref(),
        Some("hit")
    );
    assert_eq!(
        result.location_path.path_segments,
        vec!["admin".to_string()]
    );
    assert_eq!(result.location_path.conditionals.len(), 1);
}

#[test]
fn layers_error_handlers_from_matching_scopes() {
    let api_location = PreparedHostConfigurationBlock {
        directives: Arc::new(HashMap::new()),
        matches: Vec::new(),
        error_config: vec![PreparedHostConfigurationErrorConfig {
            error_code: Some(404),
            config: string_block("api_error", "yes"),
        }],
    };

    let host = PreparedHostConfigurationBlock {
        directives: Arc::new(HashMap::new()),
        matches: vec![PreparedHostConfigurationMatch {
            matcher: PreparedHostConfigurationMatcher::Location("/api".to_string()),
            config: Arc::new(api_location),
        }],
        error_config: vec![PreparedHostConfigurationErrorConfig {
            error_code: None,
            config: string_block("host_error", "yes"),
        }],
    };

    let mut hosts = HostConfigs::new();
    hosts.insert(Some("example.com".to_string()), Arc::new(host));

    let mut prepared = PreparedConfiguration::new();
    prepared.insert(Some("127.0.0.1".parse().unwrap()), hosts);

    let resolver = ThreeStageResolver::from_prepared(prepared);
    let result = resolver
        .resolve_error_scoped(
            "127.0.0.1".parse().unwrap(),
            "example.com",
            "/api/users",
            404,
            &make_test_context(empty_request(), "example.com"),
        )
        .expect("error resolution should succeed");

    assert_eq!(
        read_string(&result.configuration, "host_error").as_deref(),
        Some("yes")
    );
    assert_eq!(
        read_string(&result.configuration, "api_error").as_deref(),
        Some("yes")
    );
    assert_eq!(result.location_path.error_key, Some(404));
}
