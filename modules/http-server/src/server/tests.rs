use ferron_core::config::{
    ServerConfigurationBlock, ServerConfigurationBlockBuilder, ServerConfigurationDirectiveEntry,
    ServerConfigurationValueBuilder,
};
use ferron_observability::transform_observability_alias;

use super::*;

fn tcp_directive(children: ServerConfigurationBlock) -> ServerConfigurationDirectiveEntry {
    ServerConfigurationDirectiveEntry {
        args: vec![],
        children: Some(children),
        span: None,
    }
}

fn http_directive(children: ServerConfigurationBlock) -> ServerConfigurationDirectiveEntry {
    ServerConfigurationDirectiveEntry {
        args: vec![],
        children: Some(children),
        span: None,
    }
}

fn number_directive(value: i64) -> ServerConfigurationDirectiveEntry {
    ServerConfigurationDirectiveEntry {
        args: vec![ServerConfigurationValueBuilder::number(value)],
        children: None,
        span: None,
    }
}

fn boolean_directive(value: bool) -> ServerConfigurationDirectiveEntry {
    ServerConfigurationDirectiveEntry {
        args: vec![ServerConfigurationValueBuilder::boolean(value)],
        children: None,
        span: None,
    }
}

#[test]
fn tcp_listener_options_use_dual_stack_defaults() {
    let global_config = ServerConfigurationBlockBuilder::new().build();

    let options = resolve_tcp_listener_options(&global_config, 8080).unwrap();

    assert_eq!(
        options.address,
        SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 8080)
    );
    assert_eq!(options.send_buffer_size, None);
    assert_eq!(options.recv_buffer_size, None);
}

#[test]
fn tcp_listener_options_read_ip_and_buffer_sizes() {
    let tcp_block = ServerConfigurationBlockBuilder::new()
        .directive_str("listen", vec!["127.0.0.1"])
        .directive("send_buf", number_directive(65536))
        .directive("recv_buf", number_directive(131072))
        .build();
    let global_config = ServerConfigurationBlockBuilder::new()
        .directive("tcp", tcp_directive(tcp_block))
        .build();

    let options = resolve_tcp_listener_options(&global_config, 8080).unwrap();

    assert_eq!(
        options.address,
        SocketAddr::new(IpAddr::from([127, 0, 0, 1]), 8080)
    );
    assert_eq!(options.send_buffer_size, Some(65536));
    assert_eq!(options.recv_buffer_size, Some(131072));
}

#[test]
fn tcp_listener_options_reject_negative_buffer_sizes() {
    let tcp_block = ServerConfigurationBlockBuilder::new()
        .directive("send_buf", number_directive(-1))
        .build();
    let global_config = ServerConfigurationBlockBuilder::new()
        .directive("tcp", tcp_directive(tcp_block))
        .build();

    let error = resolve_tcp_listener_options(&global_config, 8080).unwrap_err();

    assert_eq!(
        error.to_string(),
        "tcp.send_buf must be a non-negative integer"
    );
}

#[test]
fn http_connection_options_default_to_h1_and_h2() {
    let config = ServerConfigurationBlockBuilder::new().build();

    let options = resolve_http_connection_options(&config, &config).unwrap();

    assert_eq!(options.protocols, common::HttpProtocols::default());
    assert_eq!(
        options.alpn_protocols(),
        vec![b"h2".to_vec(), b"http/1.1".to_vec(), b"http/1.0".to_vec()]
    );
    assert!(!options.h1_enable_early_hints);
    assert_eq!(options.h2, common::Http2Settings::default());
}

#[test]
fn http_connection_options_read_protocols_and_h2_settings() {
    let http_block = ServerConfigurationBlockBuilder::new()
        .directive_str("protocols", vec!["h1"])
        .directive("h1_enable_early_hints", boolean_directive(true))
        .directive("h2_initial_window_size", number_directive(65_535))
        .directive("h2_max_frame_size", number_directive(32_768))
        .directive("h2_max_concurrent_streams", number_directive(128))
        .directive("h2_max_header_list_size", number_directive(16_384))
        .directive("h2_enable_connect_protocol", boolean_directive(true))
        .build();
    let config = ServerConfigurationBlockBuilder::new()
        .directive("http", http_directive(http_block))
        .build();

    let options = resolve_http_connection_options(&config, &config).unwrap();

    assert_eq!(
        options.protocols,
        common::HttpProtocols {
            http1: true,
            http2: false,
            http3: false,
        }
    );
    assert_eq!(
        options.alpn_protocols(),
        vec![b"http/1.1".to_vec(), b"http/1.0".to_vec()]
    );
    assert!(options.h1_enable_early_hints);
    assert_eq!(
        options.h2,
        common::Http2Settings {
            initial_window_size: Some(65_535),
            max_frame_size: Some(32_768),
            max_concurrent_streams: Some(128),
            max_header_list_size: Some(16_384),
            enable_connect_protocol: true,
        }
    );
}

#[test]
fn http_connection_options_reject_unknown_protocols() {
    let http_block = ServerConfigurationBlockBuilder::new()
        .directive_str("protocols", vec!["unknown"])
        .build();
    let config = ServerConfigurationBlockBuilder::new()
        .directive("http", http_directive(http_block))
        .build();

    let error = resolve_http_connection_options(&config, &config).unwrap_err();

    assert_eq!(error.to_string(), "Unsupported HTTP protocol 'unknown'");
}

#[test]
fn transform_log_alias_with_path() {
    let log_directive = ServerConfigurationDirectiveEntry {
        args: vec![ServerConfigurationValueBuilder::string(
            "/var/log/access.log",
        )],
        children: Some(
            ServerConfigurationBlockBuilder::new()
                .directive_str("format", vec!["combined"])
                .build(),
        ),
        span: None,
    };

    let result = transform_observability_alias("log", &log_directive).unwrap();
    let block = result.expect("Should return a block");

    assert_eq!(
        block.get_value("provider").and_then(|v| v.as_str()),
        Some("file")
    );
    assert_eq!(
        block.get_value("access_log").and_then(|v| v.as_str()),
        Some("/var/log/access.log")
    );
    assert_eq!(
        block.get_value("format").and_then(|v| v.as_str()),
        Some("combined")
    );
}

#[test]
fn transform_log_alias_disabled() {
    let log_directive = ServerConfigurationDirectiveEntry {
        args: vec![ServerConfigurationValueBuilder::boolean(false)],
        children: None,
        span: None,
    };

    let result = transform_observability_alias("log", &log_directive).unwrap();
    assert!(result.is_none(), "Should return None for disabled alias");
}

#[test]
fn transform_error_log_alias_with_path() {
    let error_log_directive = ServerConfigurationDirectiveEntry {
        args: vec![ServerConfigurationValueBuilder::string(
            "/var/log/error.log",
        )],
        children: None,
        span: None,
    };

    let result = transform_observability_alias("error_log", &error_log_directive).unwrap();
    let block = result.expect("Should return a block");

    assert_eq!(
        block.get_value("provider").and_then(|v| v.as_str()),
        Some("file")
    );
    assert_eq!(
        block.get_value("error_log").and_then(|v| v.as_str()),
        Some("/var/log/error.log")
    );
}

#[test]
fn transform_console_log_alias() {
    let console_log_directive = ServerConfigurationDirectiveEntry {
        args: vec![],
        children: Some(
            ServerConfigurationBlockBuilder::new()
                .directive_str("format", vec!["json"])
                .build(),
        ),
        span: None,
    };

    let result = transform_observability_alias("console_log", &console_log_directive).unwrap();
    let block = result.expect("Should return a block");

    assert_eq!(
        block.get_value("provider").and_then(|v| v.as_str()),
        Some("console")
    );
    assert_eq!(
        block.get_value("format").and_then(|v| v.as_str()),
        Some("json")
    );
}

#[test]
fn transform_console_log_alias_disabled() {
    let console_log_directive = ServerConfigurationDirectiveEntry {
        args: vec![ServerConfigurationValueBuilder::boolean(false)],
        children: None,
        span: None,
    };

    let result = transform_observability_alias("console_log", &console_log_directive).unwrap();
    assert!(result.is_none(), "Should return None for disabled alias");
}

#[test]
fn global_observability_console_log_alias() {
    let global_config = ServerConfigurationBlockBuilder::new()
        .directive(
            "console_log",
            ServerConfigurationDirectiveEntry {
                args: vec![],
                children: Some(
                    ServerConfigurationBlockBuilder::new()
                        .directive_str("format", vec!["json"])
                        .build(),
                ),
                span: None,
            },
        )
        .build();

    let extractor = ObservabilityConfigExtractor::new(&global_config);
    let blocks = extractor.extract_observability_blocks().unwrap();

    assert_eq!(blocks.len(), 1);
    assert_eq!(
        blocks[0].get_value("provider").and_then(|v| v.as_str()),
        Some("console")
    );
    assert_eq!(
        blocks[0].get_value("format").and_then(|v| v.as_str()),
        Some("json")
    );
}

#[test]
fn global_observability_log_alias() {
    let global_config = ServerConfigurationBlockBuilder::new()
        .directive(
            "log",
            ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValueBuilder::string(
                    "/var/log/access.log",
                )],
                children: Some(
                    ServerConfigurationBlockBuilder::new()
                        .directive_str("format", vec!["combined"])
                        .build(),
                ),
                span: None,
            },
        )
        .build();

    let extractor = ObservabilityConfigExtractor::new(&global_config);
    let blocks = extractor.extract_observability_blocks().unwrap();

    assert_eq!(blocks.len(), 1);
    assert_eq!(
        blocks[0].get_value("provider").and_then(|v| v.as_str()),
        Some("file")
    );
    assert_eq!(
        blocks[0].get_value("access_log").and_then(|v| v.as_str()),
        Some("/var/log/access.log")
    );
    assert_eq!(
        blocks[0].get_value("format").and_then(|v| v.as_str()),
        Some("combined")
    );
}

#[test]
fn global_observability_error_log_alias() {
    let global_config = ServerConfigurationBlockBuilder::new()
        .directive(
            "error_log",
            ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValueBuilder::string(
                    "/var/log/error.log",
                )],
                children: None,
                span: None,
            },
        )
        .build();

    let extractor = ObservabilityConfigExtractor::new(&global_config);
    let blocks = extractor.extract_observability_blocks().unwrap();

    assert_eq!(blocks.len(), 1);
    assert_eq!(
        blocks[0].get_value("provider").and_then(|v| v.as_str()),
        Some("file")
    );
    assert_eq!(
        blocks[0].get_value("error_log").and_then(|v| v.as_str()),
        Some("/var/log/error.log")
    );
}

#[test]
fn global_observability_explicit_block() {
    let global_config = ServerConfigurationBlockBuilder::new()
        .directive(
            "observability",
            ServerConfigurationDirectiveEntry {
                args: vec![],
                children: Some(
                    ServerConfigurationBlockBuilder::new()
                        .directive_str("provider", vec!["file"])
                        .directive_str("access_log", vec!["/var/log/http.log"])
                        .build(),
                ),
                span: None,
            },
        )
        .build();

    let extractor = ObservabilityConfigExtractor::new(&global_config);
    let blocks = extractor.extract_observability_blocks().unwrap();

    assert_eq!(blocks.len(), 1);
    assert_eq!(
        blocks[0].get_value("provider").and_then(|v| v.as_str()),
        Some("file")
    );
    assert_eq!(
        blocks[0].get_value("access_log").and_then(|v| v.as_str()),
        Some("/var/log/http.log")
    );
}

#[test]
fn global_observability_multiple_aliases() {
    let global_config = ServerConfigurationBlockBuilder::new()
        .directive(
            "log",
            ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValueBuilder::string(
                    "/var/log/access.log",
                )],
                children: None,
                span: None,
            },
        )
        .directive(
            "error_log",
            ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValueBuilder::string(
                    "/var/log/error.log",
                )],
                children: None,
                span: None,
            },
        )
        .build();

    let extractor = ObservabilityConfigExtractor::new(&global_config);
    let blocks = extractor.extract_observability_blocks().unwrap();

    assert_eq!(blocks.len(), 2);
    assert_eq!(
        blocks[0].get_value("provider").and_then(|v| v.as_str()),
        Some("file")
    );
    assert_eq!(
        blocks[0].get_value("access_log").and_then(|v| v.as_str()),
        Some("/var/log/access.log")
    );
    assert_eq!(
        blocks[1].get_value("provider").and_then(|v| v.as_str()),
        Some("file")
    );
    assert_eq!(
        blocks[1].get_value("error_log").and_then(|v| v.as_str()),
        Some("/var/log/error.log")
    );
}
