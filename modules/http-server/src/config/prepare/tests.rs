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
            vec![ServerConfigurationValue::String("/var/www".into(), None)],
            None,
        ),
        (
            "index".to_string(),
            vec![ServerConfigurationValue::String("index.html".into(), None)],
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
fn test_location_directive_multiple() {
    let location1_block = create_block_with_directives(vec![(
        "proxy_pass".to_string(),
        vec![ServerConfigurationValue::String(
            "http://localhost:8080".into(),
            None,
        )],
        None,
    )]);

    let location2_block = create_block_with_directives(vec![(
        "root".to_string(),
        vec![ServerConfigurationValue::String(
            "/var/www/static".into(),
            None,
        )],
        None,
    )]);

    let block = create_block_with_directives(vec![
        (
            "location".to_string(),
            vec![ServerConfigurationValue::String("/api".into(), None)],
            Some(location1_block),
        ),
        (
            "location".to_string(),
            vec![ServerConfigurationValue::String("/static".into(), None)],
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
        vec![ServerConfigurationValue::String(
            "http://localhost:8080".into(),
            None,
        )],
        None,
    )]);

    let location2_block = create_block_with_directives(vec![(
        "proxy_set_header".to_string(),
        vec![ServerConfigurationValue::String(
            "Host localhost".into(),
            None,
        )],
        None,
    )]);

    let block = create_block_with_directives(vec![
        (
            "location".to_string(),
            vec![ServerConfigurationValue::String("/api".into(), None)],
            Some(location1_block),
        ),
        (
            "location".to_string(),
            vec![ServerConfigurationValue::String("/api".into(), None)],
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
        vec![ServerConfigurationValue::String(
            "http://localhost:8080".into(),
            None,
        )],
        None,
    )]);

    let outer_location_block = create_block_with_directives(vec![(
        "location".to_string(),
        vec![ServerConfigurationValue::String("/v1".into(), None)],
        Some(inner_location_block),
    )]);

    let block = create_block_with_directives(vec![(
        "location".to_string(),
        vec![ServerConfigurationValue::String("/api".into(), None)],
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
        ServerConfigurationMatcher {
            exprs: vec![create_eq_expr("user_agent", "Mobile")],
            span: None,
        },
    );

    let if_block = create_block_with_directives(vec![(
        "rewrite".to_string(),
        vec![ServerConfigurationValue::String("/mobile".into(), None)],
        None,
    )]);

    let mut directives_map = HashMap::new();
    directives_map.insert(
        "if".to_string(),
        vec![ServerConfigurationDirectiveEntry {
            args: vec![ServerConfigurationValue::String("is_mobile".into(), None)],
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
        vec![ServerConfigurationValue::String(
            "undefined_matcher".into(),
            None,
        )],
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
        ServerConfigurationMatcher {
            exprs: vec![create_eq_expr("user_agent", "bot")],
            span: None,
        },
    );

    let if_not_block = create_block_with_directives(vec![(
        "allow".to_string(),
        vec![ServerConfigurationValue::String("all".into(), None)],
        None,
    )]);

    let mut directives_map = HashMap::new();
    directives_map.insert(
        "if_not".to_string(),
        vec![ServerConfigurationDirectiveEntry {
            args: vec![ServerConfigurationValue::String("is_bot".into(), None)],
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
        ServerConfigurationMatcher {
            exprs: vec![create_eq_expr("scheme", "https")],
            span: None,
        },
    );

    let location_block = create_block_with_directives(vec![(
        "root".to_string(),
        vec![ServerConfigurationValue::String("/var/www".into(), None)],
        None,
    )]);

    let if_block = create_block_with_directives(vec![(
        "add_header".to_string(),
        vec![ServerConfigurationValue::String(
            "Strict-Transport-Security".into(),
            None,
        )],
        None,
    )]);

    let mut directives_map = HashMap::new();
    directives_map.insert(
        "location".to_string(),
        vec![ServerConfigurationDirectiveEntry {
            args: vec![ServerConfigurationValue::String("/".into(), None)],
            children: Some(location_block),
            span: None,
        }],
    );
    directives_map.insert(
        "if".to_string(),
        vec![ServerConfigurationDirectiveEntry {
            args: vec![ServerConfigurationValue::String("is_secure".into(), None)],
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
fn test_handle_error_without_code() {
    let error_block = create_block_with_directives(vec![(
        "root".to_string(),
        vec![ServerConfigurationValue::String("/errors".into(), None)],
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
        vec![ServerConfigurationValue::String("500".into(), None)],
        None,
    )]);

    let error2_block = create_block_with_directives(vec![(
        "add_header".to_string(),
        vec![ServerConfigurationValue::String(
            "Content-Type text/html".into(),
            None,
        )],
        None,
    )]);

    let block = create_block_with_directives(vec![
        (
            "handle_error".to_string(),
            vec![ServerConfigurationValue::Number(500, None)],
            Some(error1_block),
        ),
        (
            "handle_error".to_string(),
            vec![ServerConfigurationValue::Number(500, None)],
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
        vec![ServerConfigurationValue::String("404".into(), None)],
        None,
    )]);

    let error500_block = create_block_with_directives(vec![(
        "return".to_string(),
        vec![ServerConfigurationValue::String("500".into(), None)],
        None,
    )]);

    let block = create_block_with_directives(vec![
        (
            "handle_error".to_string(),
            vec![ServerConfigurationValue::Number(404, None)],
            Some(error404_block),
        ),
        (
            "handle_error".to_string(),
            vec![ServerConfigurationValue::Number(500, None)],
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
        vec![ServerConfigurationValue::Number(404, None)],
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
fn test_prepare_host_config_multiple_hosts() {
    let host1_block = create_block_with_directives(vec![(
        "root".to_string(),
        vec![ServerConfigurationValue::String(
            "/var/www/site1".into(),
            None,
        )],
        None,
    )]);

    let host2_block = create_block_with_directives(vec![(
        "root".to_string(),
        vec![ServerConfigurationValue::String(
            "/var/www/site2".into(),
            None,
        )],
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
        vec![ServerConfigurationValue::String("/var/www".into(), None)],
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
        vec![ServerConfigurationValue::String(
            "http://localhost:8080".into(),
            None,
        )],
        None,
    )]);

    let host_block = create_block_with_directives(vec![(
        "location".to_string(),
        vec![ServerConfigurationValue::String("/api".into(), None)],
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
        vec![ServerConfigurationValue::String("/test".into(), None)],
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
fn test_if_not_missing_children_error() {
    let mut matchers = HashMap::new();
    matchers.insert(
        "test".to_string(),
        ServerConfigurationMatcher {
            exprs: vec![create_eq_expr("foo", "bar")],
            span: None,
        },
    );

    let mut directives_map = HashMap::new();
    directives_map.insert(
        "if_not".to_string(),
        vec![ServerConfigurationDirectiveEntry {
            args: vec![ServerConfigurationValue::String("test".into(), None)],
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
