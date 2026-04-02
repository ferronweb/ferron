#![cfg(test)]

use crate::ast::*;
use std::str::FromStr;

#[test]
fn test_parser_example() {
    let input = r#"
# Global runtime settings
{
  runtime {
    io_uring true
  }
}

# Snippet
snippet set_curl {
  header X-Curl 1
}

# Matcher definition
match curl_client {
  request.header.user_agent ~ "curl"
}

# Edge case test ("true" would be classified as a boolean by the lexer here)
true.example {
  root {{env.TRUE_WWWROOT}}
}

# Default HTTP settings
http * {
  header X-Powered-By MyServer
}

# Main site
example.com {
  root /var/www/example

  if curl_client {
    use set_curl
  }
}

# Wildcard subdomains
*.example.com {
  reverse_proxy localhost:9000
}

# TCP service
tcp *:5432 {
  proxy localhost:5432
}
"#;

    let config = Config::from_str(input).expect("Failed to parse config");

    // 1. Check Global Block
    let global_block = config
        .statements
        .iter()
        .find(|s| matches!(s, Statement::GlobalBlock(_)))
        .expect("Global block not found");
    if let Statement::GlobalBlock(block) = global_block {
        let runtime = block
            .find_directive("runtime")
            .expect("runtime directive not found");
        assert!(runtime.has_block());
        let io_uring = runtime
            .block
            .as_ref()
            .unwrap()
            .find_directive("io_uring")
            .expect("io_uring not found");
        assert_eq!(io_uring.get_boolean_arg(0), Some(true));
    }

    // 2. Check Snippet
    let snippet_block = config
        .statements
        .iter()
        .find(|s| matches!(s, Statement::SnippetBlock(_)))
        .expect("Snippet block not found");
    if let Statement::SnippetBlock(sb) = snippet_block {
        assert_eq!(sb.name, "set_curl");
        let header = sb
            .block
            .find_directive("header")
            .expect("header directive not found");
        assert_eq!(header.get_string_arg(0), Some("X-Curl"));
        assert_eq!(header.get_integer_arg(1), Some(1));
    }

    // 3. Check Matcher
    let match_blocks = config.find_match_blocks();
    let curl_client = match_blocks
        .iter()
        .find(|m| m.matcher == "curl_client")
        .expect("curl_client matcher not found");
    assert!(curl_client.has_expressions());
    let expr = &curl_client.expr[0];
    assert!(expr.is_regex());
    assert_eq!(
        expr.left.as_identifier().map(|v| v.join(".")),
        Some("request.header.user_agent".to_string())
    );
    assert_eq!(expr.right.as_str(), Some("curl"));

    // 4. Check 'true.example' Host Block
    let true_example = config
        .statements
        .iter()
        .find_map(|s| {
            if let Statement::HostBlock(hb) = s {
                if hb.matches_host("true.example") {
                    Some(hb)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .expect("true.example host block not found");

    let root = true_example
        .block
        .find_directive("root")
        .expect("root directive not found");
    if let Value::InterpolatedString(parts, _) = &root.args[0] {
        assert_eq!(
            parts,
            &vec![StringPart::Expression(vec![
                "env".to_string(),
                "TRUE_WWWROOT".to_string()
            ])]
        );
    } else {
        panic!("Expected interpolation for root argument");
    }

    // 5. Check 'http *' Host Block
    let http_star = config
        .statements
        .iter()
        .find_map(|s| {
            if let Statement::HostBlock(hb) = s {
                if hb.hosts.iter().any(|h| {
                    h.protocol.as_deref() == Some("http")
                        && h.labels == crate::ast::HostLabels::Wildcard
                }) {
                    Some(hb)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .expect("http * host block not found");

    let powered_by = http_star
        .block
        .find_directive("header")
        .expect("header directive not found");
    assert_eq!(powered_by.get_string_arg(0), Some("X-Powered-By"));
    assert_eq!(powered_by.get_string_arg(1), Some("MyServer"));

    // 6. Check 'example.com' Host Block
    let example_com = config
        .statements
        .iter()
        .find_map(|s| {
            if let Statement::HostBlock(hb) = s {
                if hb.matches_host("example.com") {
                    Some(hb)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .expect("example.com host block not found");

    let root = example_com
        .block
        .find_directive("root")
        .expect("root directive not found");
    assert_eq!(root.get_string_arg(0), Some("/var/www/example"));

    let if_directive = example_com
        .block
        .find_directive("if")
        .expect("if directive not found");
    assert_eq!(if_directive.args[0].as_str(), Some("curl_client"));

    // 7. Check TCP service
    let tcp_service = config
        .statements
        .iter()
        .find_map(|s| {
            if let Statement::HostBlock(hb) = s {
                if hb
                    .hosts
                    .iter()
                    .any(|h| h.protocol.as_deref() == Some("tcp") && h.port == Some(5432))
                {
                    Some(hb)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .expect("tcp *:5432 host block not found");

    let proxy = tcp_service
        .block
        .find_directive("proxy")
        .expect("proxy directive not found");
    assert_eq!(proxy.get_string_arg(0), Some("localhost:5432"));
}

#[test]
fn test_complex_values() {
    let input = r#"
directive_float 3.14
directive_neg -10
directive_neg_float -3.14
directive_string "string with \"escape\""
directive_interp {{ nested.var }}
directive_interp_multi "prefix {{ nested.var }} suffix {{ other.value }}"
directive_bools true false
"#;
    let config = Config::from_str(input).expect("Failed to parse complex values");

    // Float
    let d_float = config.find_directives("directive_float")[0];
    assert_eq!(d_float.args[0].as_f64(), Some(3.14));

    // Negative Number
    let d_neg = config.find_directives("directive_neg")[0];
    assert_eq!(d_neg.args[0].as_i64(), Some(-10));

    // Negative Float
    let d_neg_float = config.find_directives("directive_neg_float")[0];
    assert_eq!(d_neg_float.args[0].as_f64(), Some(-3.14));

    // String Escapes
    let d_str = config.find_directives("directive_string")[0];
    assert_eq!(d_str.args[0].as_str(), Some("string with \"escape\""));

    // Interpolation
    let d_interp = config.find_directives("directive_interp")[0];
    assert_eq!(
        d_interp.args[0].as_interpolated_string(),
        Some(&[StringPart::Expression(vec![
            "nested".to_string(),
            "var".to_string()
        ])] as &[StringPart])
    );

    let d_interp_multi = config.find_directives("directive_interp_multi")[0];
    assert_eq!(
        d_interp_multi.args[0].as_interpolated_string(),
        Some(&[
            StringPart::Literal("prefix ".to_string()),
            StringPart::Expression(vec!["nested".to_string(), "var".to_string()]),
            StringPart::Literal(" suffix ".to_string()),
            StringPart::Expression(vec!["other".to_string(), "value".to_string()]),
        ] as &[StringPart])
    );

    // Booleans
    let d_bool = config.find_directives("directive_bools")[0];
    assert_eq!(d_bool.get_boolean_arg(0), Some(true));
    assert_eq!(d_bool.get_boolean_arg(1), Some(false));
}

#[test]
fn test_host_patterns() {
    let input = r#"
[::1] {}
[2001:db8::1]:8080 {}
127.0.0.1 {}
"#;
    let config = Config::from_str(input).expect("Failed to parse host patterns");

    config
        .statements
        .iter()
        .find_map(|s| {
            if let Statement::HostBlock(hb) = s {
                if hb.hosts[0].labels
                    == crate::ast::HostLabels::IpAddr(std::net::IpAddr::V6(
                        std::net::Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1),
                    ))
                {
                    Some(hb)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .expect("IPv6 localhost not found");

    let ipv6_port = config
        .statements
        .iter()
        .find_map(|s| {
            if let Statement::HostBlock(hb) = s {
                if hb.hosts[0].port == Some(8080) {
                    Some(hb)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .expect("IPv6 with port not found");

    // Check IPv6 address explicitly
    if let crate::ast::HostLabels::IpAddr(std::net::IpAddr::V6(addr)) = &ipv6_port.hosts[0].labels {
        assert_eq!(addr.to_string(), "2001:db8::1");
    } else {
        panic!("Expected IPv6 address");
    }

    let ipv4 = config
        .statements
        .iter()
        .find_map(|s| {
            if let Statement::HostBlock(hb) = s {
                if hb.hosts[0].as_str() == "127.0.0.1" {
                    Some(hb)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .expect("IPv4 not found");
    assert!(matches!(
        ipv4.hosts[0].labels,
        crate::ast::HostLabels::IpAddr(std::net::IpAddr::V4(_))
    ));
}

#[test]
fn test_top_level_ambiguity() {
    // Case 1: Directive with Quoted String and Block.
    let input_quoted = r#"
    dir_quoted "arg" {
        inside true
    }
    "#;
    let config = Config::from_str(input_quoted).expect("dir_quoted should parse successfully now");
    if let Statement::Directive(d) = &config.statements[0] {
        assert_eq!(d.name, "dir_quoted");
        assert_eq!(d.args[0].as_str(), Some("arg"));
        assert!(d.has_block());
    } else {
        panic!("dir_quoted did not parse as Directive");
    }

    // Case 2: Directive with Bare String and Block.
    // Parses as HostBlock.
    let input_bare = r#"
    dir_bare arg { }
    "#;
    let config = Config::from_str(input_bare).expect("dir_bare failed");
    if let Statement::HostBlock(hb) = &config.statements[0] {
        assert_eq!(hb.hosts[0].protocol.as_deref(), Some("dir_bare"));
        // "arg" is parsed as part of the host label sequence.
        // wait, parse_host_pattern consumes "dir_bare" then "arg".
        // labels=["dir_bare", "arg"].
        // then it sees protocol is None.
        // "dir_bare" becomes protocol. "arg" stays in labels.
        // Correct.
        assert_eq!(hb.hosts[0].as_str(), "arg");
    } else {
        panic!("dir_bare did not parse as HostBlock");
    }
}
