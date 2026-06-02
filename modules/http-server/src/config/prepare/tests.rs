use super::*;
use ferron_core::config::{
    ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationHostFilters,
    ServerConfigurationMatcher, ServerConfigurationMatcherExpr, ServerConfigurationMatcherOperand,
    ServerConfigurationMatcherOperator, ServerConfigurationPort, ServerConfigurationValue,
};

// Helper functions to create test configuration blocks
fn create_block_with_directives(
    directives: Vec<(
        String,
        Vec<ServerConfigurationValue>,
        Option<ServerConfigurationBlock>,
    )>,
) -> ServerConfigurationBlock {
    let mut directive_map: HashMap<String, Vec<ServerConfigurationDirectiveEntry>> = HashMap::new();

    for (name, args, children) in directives {
        let entry = ServerConfigurationDirectiveEntry {
            args,
            children,
            span: None,
        };
        directive_map.entry(name).or_default().push(entry);
    }

    ServerConfigurationBlock {
        directives: Arc::new(directive_map),
        matchers: HashMap::new(),
        span: None,
    }
}

fn create_string_value(s: &str) -> ServerConfigurationValue {
    ServerConfigurationValue::String(s.to_string(), None)
}

fn create_number_value(n: i64) -> ServerConfigurationValue {
    ServerConfigurationValue::Number(n, None)
}

fn create_matcher(
    _name: &str,
    exprs: Vec<ServerConfigurationMatcherExpr>,
) -> ServerConfigurationMatcher {
    ServerConfigurationMatcher { exprs, span: None }
}

fn create_eq_expr(identifier: &str, value: &str) -> ServerConfigurationMatcherExpr {
    ServerConfigurationMatcherExpr {
        left: ServerConfigurationMatcherOperand::Identifier(identifier.to_string()),
        right: ServerConfigurationMatcherOperand::String(value.to_string()),
        op: ServerConfigurationMatcherOperator::Eq,
    }
}

#[test]
fn test_empty_block() {
    let block = ServerConfigurationBlock {
        directives: Arc::new(HashMap::new()),
        matchers: HashMap::new(),
        span: None,
    };

    let result = prepare_host_block(block).unwrap();

    assert!(result.directives.is_empty());
    assert!(result.matches.is_empty());
    assert!(result.error_config.is_empty());
}

#[test]
fn test_block_with_simple_directives() {
    let block = create_block_with_directives(vec![
        (
            "root".to_string(),
            vec![create_string_value("/var/www")],
            None,
        ),
        (
            "index".to_string(),
            vec![create_string_value("index.html")],
            None,
        ),
    ]);

    let result = prepare_host_block(block).unwrap();

    assert!(result.directives.contains_key("root"));
    assert!(result.directives.contains_key("index"));
    assert!(result.matches.is_empty());
    assert!(result.error_config.is_empty());
}

#[test]
fn test_location_directive_single() {
    let location_block = create_block_with_directives(vec![(
        "proxy_pass".to_string(),
        vec![create_string_value("http://localhost:8080")],
        None,
    )]);

    let block = create_block_with_directives(vec![(
        "location".to_string(),
        vec![create_string_value("/api")],
        Some(location_block),
    )]);

    let result = prepare_host_block(block).unwrap();

    assert!(!result.directives.contains_key("location"));
    assert_eq!(result.matches.len(), 1);

    let location_match = &result.matches[0];
    match &location_match.matcher {
        PreparedHostConfigurationMatcher::Location(path) => {
            assert_eq!(path, "/api");
        }
        _ => panic!("Expected Location matcher"),
    }

    assert!(location_match.config.directives.contains_key("proxy_pass"));
}

#[test]
fn test_location_directive_multiple() {
    let location1_block = create_block_with_directives(vec![(
        "proxy_pass".to_string(),
        vec![create_string_value("http://localhost:8080")],
        None,
    )]);

    let location2_block = create_block_with_directives(vec![(
        "root".to_string(),
        vec![create_string_value("/var/www/static")],
        None,
    )]);

    let block = create_block_with_directives(vec![
        (
            "location".to_string(),
            vec![create_string_value("/api")],
            Some(location1_block),
        ),
        (
            "location".to_string(),
            vec![create_string_value("/static")],
            Some(location2_block),
        ),
    ]);

    let result = prepare_host_block(block).unwrap();

    assert_eq!(result.matches.len(), 2);

    let locations: Vec<&str> = result
        .matches
        .iter()
        .filter_map(|m| match &m.matcher {
            PreparedHostConfigurationMatcher::Location(path) => Some(path.as_str()),
            _ => None,
        })
        .collect();

    assert!(locations.contains(&"/api"));
    assert!(locations.contains(&"/static"));
}

#[test]
fn test_location_directive_duplicate_merged() {
    let location1_block = create_block_with_directives(vec![(
        "proxy_pass".to_string(),
        vec![create_string_value("http://localhost:8080")],
        None,
    )]);

    let location2_block = create_block_with_directives(vec![(
        "proxy_set_header".to_string(),
        vec![create_string_value("Host localhost")],
        None,
    )]);

    let block = create_block_with_directives(vec![
        (
            "location".to_string(),
            vec![create_string_value("/api")],
            Some(location1_block),
        ),
        (
            "location".to_string(),
            vec![create_string_value("/api")],
            Some(location2_block),
        ),
    ]);

    let result = prepare_host_block(block).unwrap();

    assert_eq!(result.matches.len(), 1);

    let location_match = &result.matches[0];
    assert!(location_match.config.directives.contains_key("proxy_pass"));
    assert!(location_match
        .config
        .directives
        .contains_key("proxy_set_header"));
}

#[test]
fn test_location_directive_nested_locations() {
    let inner_location_block = create_block_with_directives(vec![(
        "proxy_pass".to_string(),
        vec![create_string_value("http://localhost:8080")],
        None,
    )]);

    let outer_location_block = create_block_with_directives(vec![(
        "location".to_string(),
        vec![create_string_value("/v1")],
        Some(inner_location_block),
    )]);

    let block = create_block_with_directives(vec![(
        "location".to_string(),
        vec![create_string_value("/api")],
        Some(outer_location_block),
    )]);

    let result = prepare_host_block(block).unwrap();

    assert_eq!(result.matches.len(), 1);
    let outer_match = &result.matches[0];

    match &outer_match.matcher {
        PreparedHostConfigurationMatcher::Location(path) => {
            assert_eq!(path, "/api");
        }
        _ => panic!("Expected Location matcher"),
    }

    assert_eq!(outer_match.config.matches.len(), 1);
    let inner_match = &outer_match.config.matches[0];

    match &inner_match.matcher {
        PreparedHostConfigurationMatcher::Location(path) => {
            assert_eq!(path, "/v1");
        }
        _ => panic!("Expected Location matcher"),
    }
}

#[test]
fn test_if_directive_single() {
    let mut matchers = HashMap::new();
    matchers.insert(
        "is_mobile".to_string(),
        create_matcher("is_mobile", vec![create_eq_expr("user_agent", "Mobile")]),
    );

    let if_block = create_block_with_directives(vec![(
        "rewrite".to_string(),
        vec![create_string_value("/mobile")],
        None,
    )]);

    let mut directives_map = HashMap::new();
    directives_map.insert(
        "if".to_string(),
        vec![ServerConfigurationDirectiveEntry {
            args: vec![create_string_value("is_mobile")],
            children: Some(if_block),
            span: None,
        }],
    );
    let block = ServerConfigurationBlock {
        directives: Arc::new(directives_map),
        matchers,
        span: None,
    };

    let result = prepare_host_block(block).unwrap();

    assert_eq!(result.matches.len(), 1);

    match &result.matches[0].matcher {
        PreparedHostConfigurationMatcher::IfConditional(exprs) => {
            assert_eq!(exprs.len(), 1);
            assert_eq!(exprs[0].op, ServerConfigurationMatcherOperator::Eq);
        }
        _ => panic!("Expected IfConditional matcher"),
    }
}

#[test]
fn test_if_directive_undefined_matcher_error() {
    let block = create_block_with_directives(vec![(
        "if".to_string(),
        vec![create_string_value("undefined_matcher")],
        Some(create_block_with_directives(vec![])),
    )]);

    let result = prepare_host_block(block);

    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Undefined matcher"));
}

#[test]
fn test_if_not_directive_single() {
    let mut matchers = HashMap::new();
    matchers.insert(
        "is_bot".to_string(),
        create_matcher("is_bot", vec![create_eq_expr("user_agent", "bot")]),
    );

    let if_not_block = create_block_with_directives(vec![(
        "allow".to_string(),
        vec![create_string_value("all")],
        None,
    )]);

    let mut directives_map = HashMap::new();
    directives_map.insert(
        "if_not".to_string(),
        vec![ServerConfigurationDirectiveEntry {
            args: vec![create_string_value("is_bot")],
            children: Some(if_not_block),
            span: None,
        }],
    );
    let block = ServerConfigurationBlock {
        directives: Arc::new(directives_map),
        matchers,
        span: None,
    };

    let result = prepare_host_block(block).unwrap();

    assert_eq!(result.matches.len(), 1);

    match &result.matches[0].matcher {
        PreparedHostConfigurationMatcher::IfNotConditional(exprs) => {
            assert_eq!(exprs.len(), 1);
        }
        _ => panic!("Expected IfNotConditional matcher"),
    }
}

#[test]
fn test_mixed_location_and_conditional_matches() {
    let mut matchers = HashMap::new();
    matchers.insert(
        "is_secure".to_string(),
        create_matcher("is_secure", vec![create_eq_expr("scheme", "https")]),
    );

    let location_block = create_block_with_directives(vec![(
        "root".to_string(),
        vec![create_string_value("/var/www")],
        None,
    )]);

    let if_block = create_block_with_directives(vec![(
        "add_header".to_string(),
        vec![create_string_value("Strict-Transport-Security")],
        None,
    )]);

    let mut directives_map = HashMap::new();
    directives_map.insert(
        "location".to_string(),
        vec![ServerConfigurationDirectiveEntry {
            args: vec![create_string_value("/")],
            children: Some(location_block),
            span: None,
        }],
    );
    directives_map.insert(
        "if".to_string(),
        vec![ServerConfigurationDirectiveEntry {
            args: vec![create_string_value("is_secure")],
            children: Some(if_block),
            span: None,
        }],
    );
    let block = ServerConfigurationBlock {
        directives: Arc::new(directives_map),
        matchers,
        span: None,
    };

    let result = prepare_host_block(block).unwrap();

    assert_eq!(result.matches.len(), 2);

    let has_location = result.matches.iter().any(
        |m| matches!(m.matcher, PreparedHostConfigurationMatcher::Location(ref p) if p == "/"),
    );
    let has_if = result.matches.iter().any(|m| {
        matches!(
            m.matcher,
            PreparedHostConfigurationMatcher::IfConditional(_)
        )
    });

    assert!(has_location);
    assert!(has_if);
}

#[test]
fn test_handle_error_single() {
    let error_block = create_block_with_directives(vec![(
        "return".to_string(),
        vec![create_string_value("404")],
        None,
    )]);

    let block = create_block_with_directives(vec![(
        "handle_error".to_string(),
        vec![create_number_value(404)],
        Some(error_block),
    )]);

    let result = prepare_host_block(block).unwrap();

    assert_eq!(result.error_config.len(), 1);
    assert_eq!(result.error_config[0].error_code, Some(404));
    assert!(result.error_config[0]
        .config
        .directives
        .contains_key("return"));
}

#[test]
fn test_handle_error_without_code() {
    let error_block = create_block_with_directives(vec![(
        "root".to_string(),
        vec![create_string_value("/errors")],
        None,
    )]);

    let block = create_block_with_directives(vec![(
        "handle_error".to_string(),
        vec![],
        Some(error_block),
    )]);

    let result = prepare_host_block(block).unwrap();

    assert_eq!(result.error_config.len(), 1);
    assert_eq!(result.error_config[0].error_code, None);
}

#[test]
fn test_handle_error_duplicate_merged() {
    let error1_block = create_block_with_directives(vec![(
        "return".to_string(),
        vec![create_string_value("500")],
        None,
    )]);

    let error2_block = create_block_with_directives(vec![(
        "add_header".to_string(),
        vec![create_string_value("Content-Type text/html")],
        None,
    )]);

    let block = create_block_with_directives(vec![
        (
            "handle_error".to_string(),
            vec![create_number_value(500)],
            Some(error1_block),
        ),
        (
            "handle_error".to_string(),
            vec![create_number_value(500)],
            Some(error2_block),
        ),
    ]);

    let result = prepare_host_block(block).unwrap();

    assert_eq!(result.error_config.len(), 1);
    assert!(result.error_config[0]
        .config
        .directives
        .contains_key("return"));
    assert!(result.error_config[0]
        .config
        .directives
        .contains_key("add_header"));
}

#[test]
fn test_handle_error_multiple_codes() {
    let error404_block = create_block_with_directives(vec![(
        "return".to_string(),
        vec![create_string_value("404")],
        None,
    )]);

    let error500_block = create_block_with_directives(vec![(
        "return".to_string(),
        vec![create_string_value("500")],
        None,
    )]);

    let block = create_block_with_directives(vec![
        (
            "handle_error".to_string(),
            vec![create_number_value(404)],
            Some(error404_block),
        ),
        (
            "handle_error".to_string(),
            vec![create_number_value(500)],
            Some(error500_block),
        ),
    ]);

    let result = prepare_host_block(block).unwrap();

    assert_eq!(result.error_config.len(), 2);

    let codes: Vec<Option<u16>> = result.error_config.iter().map(|e| e.error_code).collect();
    assert!(codes.contains(&Some(404)));
    assert!(codes.contains(&Some(500)));
}

#[test]
fn test_handle_error_missing_block_error() {
    let block = create_block_with_directives(vec![(
        "handle_error".to_string(),
        vec![create_number_value(404)],
        None,
    )]);

    let result = prepare_host_block(block);

    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Error directive must have a block"));
}

#[test]
fn test_prepare_host_config_empty() {
    let port = ServerConfigurationPort {
        port: Some(80),
        hosts: vec![],
    };

    let result = prepare_host_config(port).unwrap();
    assert!(result.is_empty());
}

#[test]
fn test_prepare_host_config_single_host() {
    let host_block = create_block_with_directives(vec![(
        "root".to_string(),
        vec![create_string_value("/var/www")],
        None,
    )]);

    let port = ServerConfigurationPort {
        port: Some(80),
        hosts: vec![(
            ServerConfigurationHostFilters {
                ip: None,
                host: Some("example.com".to_string()),
            },
            host_block,
        )],
    };

    let result = prepare_host_config(port).unwrap();

    assert_eq!(result.len(), 1);
    assert!(result.contains_key(&None));

    let host_configs = result.get(&None).unwrap();
    assert_eq!(host_configs.named_hosts.len(), 1);
    assert!(host_configs.named_hosts.contains_key("example.com"));
}

#[test]
fn test_prepare_host_config_multiple_hosts() {
    let host1_block = create_block_with_directives(vec![(
        "root".to_string(),
        vec![create_string_value("/var/www/site1")],
        None,
    )]);

    let host2_block = create_block_with_directives(vec![(
        "root".to_string(),
        vec![create_string_value("/var/www/site2")],
        None,
    )]);

    let port = ServerConfigurationPort {
        port: Some(80),
        hosts: vec![
            (
                ServerConfigurationHostFilters {
                    ip: None,
                    host: Some("site1.com".to_string()),
                },
                host1_block,
            ),
            (
                ServerConfigurationHostFilters {
                    ip: None,
                    host: Some("site2.com".to_string()),
                },
                host2_block,
            ),
        ],
    };

    let result = prepare_host_config(port).unwrap();

    assert_eq!(result.len(), 1);
    let host_configs = result.get(&None).unwrap();
    assert_eq!(host_configs.named_hosts.len(), 2);
}

#[test]
fn test_prepare_host_config_with_ip() {
    use std::net::Ipv4Addr;

    let host_block = create_block_with_directives(vec![(
        "root".to_string(),
        vec![create_string_value("/var/www")],
        None,
    )]);

    let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));

    let port = ServerConfigurationPort {
        port: Some(80),
        hosts: vec![(
            ServerConfigurationHostFilters {
                ip: Some(ip),
                host: Some("example.com".to_string()),
            },
            host_block,
        )],
    };

    let result = prepare_host_config(port).unwrap();

    assert_eq!(result.len(), 1);
    assert!(result.contains_key(&Some(ip)));
}

#[test]
fn test_prepare_host_config_complex() {
    use std::net::Ipv4Addr;

    let ip1 = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));
    let ip2 = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 2));

    let location_block = create_block_with_directives(vec![(
        "proxy_pass".to_string(),
        vec![create_string_value("http://localhost:8080")],
        None,
    )]);

    let host_block = create_block_with_directives(vec![(
        "location".to_string(),
        vec![create_string_value("/api")],
        Some(location_block),
    )]);

    let port = ServerConfigurationPort {
        port: Some(80),
        hosts: vec![
            (
                ServerConfigurationHostFilters {
                    ip: Some(ip1),
                    host: Some("api.example.com".to_string()),
                },
                host_block.clone(),
            ),
            (
                ServerConfigurationHostFilters {
                    ip: Some(ip2),
                    host: Some("web.example.com".to_string()),
                },
                host_block,
            ),
        ],
    };

    let result = prepare_host_config(port).unwrap();

    assert_eq!(result.len(), 2);

    for host_configs in result.values() {
        for config in host_configs.named_hosts.values() {
            assert_eq!(config.matches.len(), 1);
            match &config.matches[0].matcher {
                PreparedHostConfigurationMatcher::Location(path) => {
                    assert_eq!(path, "/api");
                }
                _ => panic!("Expected Location matcher"),
            }
        }
    }
}

#[test]
fn test_location_missing_children_error() {
    let block = create_block_with_directives(vec![(
        "location".to_string(),
        vec![create_string_value("/test")],
        None,
    )]);

    let result = prepare_host_block(block);

    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Location directive must have a block"));
}

#[test]
fn test_if_missing_children_error() {
    let mut matchers = HashMap::new();
    matchers.insert(
        "test".to_string(),
        create_matcher("test", vec![create_eq_expr("foo", "bar")]),
    );

    let mut directives_map = HashMap::new();
    directives_map.insert(
        "if".to_string(),
        vec![ServerConfigurationDirectiveEntry {
            args: vec![create_string_value("test")],
            children: None,
            span: None,
        }],
    );
    let block = ServerConfigurationBlock {
        directives: Arc::new(directives_map),
        matchers,
        span: None,
    };

    let result = prepare_host_block(block);

    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("`if` directive must have a block"));
}

#[test]
fn test_if_not_missing_children_error() {
    let mut matchers = HashMap::new();
    matchers.insert(
        "test".to_string(),
        create_matcher("test", vec![create_eq_expr("foo", "bar")]),
    );

    let mut directives_map = HashMap::new();
    directives_map.insert(
        "if_not".to_string(),
        vec![ServerConfigurationDirectiveEntry {
            args: vec![create_string_value("test")],
            children: None,
            span: None,
        }],
    );
    let block = ServerConfigurationBlock {
        directives: Arc::new(directives_map),
        matchers,
        span: None,
    };

    let result = prepare_host_block(block);

    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("`if_not` directive must have a block"));
}
