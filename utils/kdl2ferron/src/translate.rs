use std::collections::{HashMap, VecDeque};

use crate::read_kdl::read_kdl_file;

pub fn translate(
    doc: &kdlite::dom::Document<'static>,
    snippets: &mut HashMap<String, kdlite::dom::Document<'static>>,
) -> Result<ferronconf::Config, anyhow::Error> {
    let mut config = ferronconf::Config { statements: vec![] };

    for node in &doc.nodes {
        match node.name() {
            "include" => {
                let value = node
                    .entries
                    .first()
                    .and_then(|e| match &e.value {
                        kdlite::dom::Value::String(value) => Some(value),
                        _ => None,
                    })
                    .ok_or(anyhow::anyhow!("include node must have a value"))?;
                let glob_found = glob::glob(value)
                    .map_err(|e| anyhow::anyhow!("can't parse glob pattern: {}", e))?;
                for path in glob_found {
                    let kdl_document = read_kdl_file(
                        path.map_err(|e| anyhow::anyhow!("can't read file: {}", e))?
                            .as_path(),
                    )
                    .map_err(|e| anyhow::anyhow!("can't parse kdl file: {}", e))?;
                    config
                        .statements
                        .extend(translate(&kdl_document, snippets)?.statements);
                }
            }
            "snippet" => {
                let value = node
                    .entries
                    .first()
                    .and_then(|e| match &e.value {
                        kdlite::dom::Value::String(value) => Some(value),
                        _ => None,
                    })
                    .ok_or(anyhow::anyhow!("snippet node must have a value"))?;
                if let Some(kdl_document) = node.children.clone() {
                    snippets.insert(value.to_string(), kdl_document);
                }
            }
            "use" => {
                let value = node
                    .entries
                    .first()
                    .and_then(|e| match &e.value {
                        kdlite::dom::Value::String(value) => Some(value),
                        _ => None,
                    })
                    .ok_or(anyhow::anyhow!("snippet node must have a value"))?;
                if let Some(snippet) = snippets.get(&value.to_string()).cloned() {
                    config
                        .statements
                        .extend(translate(&snippet, snippets)?.statements);
                } else {
                    return Err(anyhow::anyhow!("snippet not found: {}", value));
                }
            }
            block_scope => {
                let block = process_block(
                    node.children
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("block node must have children"))?,
                    snippets,
                )?;

                let stmt = if block_scope == "globals" {
                    ferronconf::Statement::GlobalBlock(block)
                } else {
                    ferronconf::Statement::HostBlock(ferronconf::HostBlock {
                        hosts: block_scope
                            .split(",")
                            .map(|scope| {
                                let host_port = scope.rsplit_once(':').and_then(|(host, port)| {
                                    port.parse::<u16>().ok().map(|port| (host, port))
                                });
                                ferronconf::HostPattern {
                                    labels: {
                                        let host = host_port.map(|(host, _)| host).unwrap_or(scope);
                                        if host == "*" || host.is_empty() {
                                            ferronconf::HostLabels::Wildcard
                                        } else if let Ok(ip) = host.parse::<std::net::IpAddr>() {
                                            ferronconf::HostLabels::IpAddr(ip)
                                        } else {
                                            ferronconf::HostLabels::Hostname(
                                                host.trim_matches('.')
                                                    .split('.')
                                                    .map(ToString::to_string)
                                                    .collect::<Vec<_>>(),
                                            )
                                        }
                                    },
                                    port: host_port.map(|(_, port)| port),
                                    protocol: None, // Implicit default "http"
                                    span: ferronconf::Span { line: 0, column: 0 },
                                }
                            })
                            .collect::<Vec<_>>(),
                        block,
                        span: ferronconf::Span { line: 0, column: 0 },
                    })
                };
                config.statements.push(stmt);
            }
        }
    }

    Ok(config)
}

/// Process a block and translate to Ferron 3 configuration
pub fn process_block(
    block: &kdlite::dom::Document<'static>,
    snippets: &HashMap<String, kdlite::dom::Document<'static>>,
) -> Result<ferronconf::Block, anyhow::Error> {
    let mut statements = Vec::new();
    let mut nested_directives: HashMap<&'static str, ferronconf::Block> = HashMap::new();
    let mut date_format: Option<String> = None; // `log_date_format`

    for node in &block.nodes {
        match node.name() {
            // Security & TLS
            "tls" => {
                let cert = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                let key = node.entries.get(1).and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });

                let mut tls_block = ferronconf::Block {
                    statements: vec![],
                    span: ferronconf::Span { line: 0, column: 0 },
                };
                if let (Some(cert), Some(key)) = (cert, key) {
                    tls_block.statements.push(ferronconf::Statement::Directive(
                        ferronconf::Directive {
                            name: "provider".to_string(),
                            args: vec![ferronconf::Value::String(
                                "manual".to_string(),
                                ferronconf::Span { line: 0, column: 0 },
                            )],
                            block: None,
                            span: ferronconf::Span { line: 0, column: 0 },
                        },
                    ));
                    tls_block.statements.push(ferronconf::Statement::Directive(
                        ferronconf::Directive {
                            name: "cert".to_string(),
                            args: vec![ferronconf::Value::String(
                                cert,
                                ferronconf::Span { line: 0, column: 0 },
                            )],
                            block: None,
                            span: ferronconf::Span { line: 0, column: 0 },
                        },
                    ));
                    tls_block.statements.push(ferronconf::Statement::Directive(
                        ferronconf::Directive {
                            name: "key".to_string(),
                            args: vec![ferronconf::Value::String(
                                key,
                                ferronconf::Span { line: 0, column: 0 },
                            )],
                            block: None,
                            span: ferronconf::Span { line: 0, column: 0 },
                        },
                    ));
                }
                statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                    name: "tls".to_string(),
                    args: vec![],
                    block: Some(tls_block),
                    span: ferronconf::Span { line: 0, column: 0 },
                }));
            }
            "auto_tls" => {
                let enabled = node.entries.first().is_none_or(|e| match e.value {
                    kdlite::dom::Value::Bool(b) => b,
                    _ => true,
                });
                if enabled {
                    nested_directives
                        .entry("tls")
                        .or_insert_with(|| ferronconf::Block {
                            statements: vec![],
                            span: ferronconf::Span { line: 0, column: 0 },
                        })
                        .statements
                        .push(ferronconf::Statement::Directive(ferronconf::Directive {
                            name: "provider".to_string(),
                            args: vec![ferronconf::Value::String(
                                "acme".to_string(),
                                ferronconf::Span { line: 0, column: 0 },
                            )],
                            block: None,
                            span: ferronconf::Span { line: 0, column: 0 },
                        }));
                }
            }
            "auto_tls_contact" => {
                let contact = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                if let Some(contact) = contact {
                    nested_directives
                        .entry("tls")
                        .or_insert_with(|| ferronconf::Block {
                            statements: vec![],
                            span: ferronconf::Span { line: 0, column: 0 },
                        })
                        .statements
                        .push(ferronconf::Statement::Directive(ferronconf::Directive {
                            name: "contact".to_string(),
                            args: vec![ferronconf::Value::String(
                                contact,
                                ferronconf::Span { line: 0, column: 0 },
                            )],
                            block: None,
                            span: ferronconf::Span { line: 0, column: 0 },
                        }));
                }
            }
            "auto_tls_cache" => {
                let cache = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                if let Some(cache) = cache {
                    nested_directives
                        .entry("tls")
                        .or_insert_with(|| ferronconf::Block {
                            statements: vec![],
                            span: ferronconf::Span { line: 0, column: 0 },
                        })
                        .statements
                        .push(ferronconf::Statement::Directive(ferronconf::Directive {
                            name: "cache".to_string(),
                            args: vec![ferronconf::Value::String(
                                cache,
                                ferronconf::Span { line: 0, column: 0 },
                            )],
                            block: None,
                            span: ferronconf::Span { line: 0, column: 0 },
                        }));
                }
            }
            "auto_tls_letsencrypt_production" | "auto_tls_directory" => {
                let val = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    kdlite::dom::Value::Bool(b) => Some(if *b {
                        "https://acme-v02.api.letsencrypt.org/directory".to_string()
                    } else {
                        "https://acme-staging-v02.api.letsencrypt.org/directory".to_string()
                    }),
                    _ => None,
                });
                if let Some(val) = val {
                    nested_directives
                        .entry("tls")
                        .or_insert_with(|| ferronconf::Block {
                            statements: vec![],
                            span: ferronconf::Span { line: 0, column: 0 },
                        })
                        .statements
                        .push(ferronconf::Statement::Directive(ferronconf::Directive {
                            name: "directory".to_string(),
                            args: vec![ferronconf::Value::String(
                                val,
                                ferronconf::Span { line: 0, column: 0 },
                            )],
                            block: None,
                            span: ferronconf::Span { line: 0, column: 0 },
                        }));
                }
            }
            "auto_tls_challenge" => {
                let challenge = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                if let Some(challenge) = challenge {
                    nested_directives
                        .entry("tls")
                        .or_insert_with(|| ferronconf::Block {
                            statements: vec![],
                            span: ferronconf::Span { line: 0, column: 0 },
                        })
                        .statements
                        .push(ferronconf::Statement::Directive(ferronconf::Directive {
                            name: "challenge".to_string(),
                            args: vec![ferronconf::Value::String(
                                challenge,
                                ferronconf::Span { line: 0, column: 0 },
                            )],
                            block: None,
                            span: ferronconf::Span { line: 0, column: 0 },
                        }));
                }
            }
            "auto_tls_no_verification"
            | "auto_tls_profile"
            | "auto_tls_on_demand"
            | "auto_tls_on_demand_ask"
            | "auto_tls_on_demand_ask_no_verification" => {
                let name = match node.name() {
                    "auto_tls_no_verification" => "no_verification",
                    "auto_tls_profile" => "profile",
                    "auto_tls_on_demand" => "on_demand",
                    "auto_tls_on_demand_ask" => "on_demand_ask",
                    "auto_tls_on_demand_ask_no_verification" => "on_demand_ask_no_verification",
                    _ => unreachable!(),
                };
                let val = node.entries.first().map(|e| match &e.value {
                    kdlite::dom::Value::String(s) => ferronconf::Value::String(
                        s.to_string(),
                        ferronconf::Span { line: 0, column: 0 },
                    ),
                    kdlite::dom::Value::Bool(b) => {
                        ferronconf::Value::Boolean(*b, ferronconf::Span { line: 0, column: 0 })
                    }
                    _ => ferronconf::Value::Boolean(true, ferronconf::Span { line: 0, column: 0 }),
                });
                if let Some(val) = val {
                    nested_directives
                        .entry("tls")
                        .or_insert_with(|| ferronconf::Block {
                            statements: vec![],
                            span: ferronconf::Span { line: 0, column: 0 },
                        })
                        .statements
                        .push(ferronconf::Statement::Directive(ferronconf::Directive {
                            name: name.to_string(),
                            args: vec![val],
                            block: None,
                            span: ferronconf::Span { line: 0, column: 0 },
                        }));
                }
            }
            "auto_tls_eab" => {
                let key_id = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                let hmac = node.entries.get(1).and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                if let (Some(key_id), Some(hmac)) = (key_id, hmac) {
                    nested_directives
                        .entry("tls")
                        .or_insert_with(|| ferronconf::Block {
                            statements: vec![],
                            span: ferronconf::Span { line: 0, column: 0 },
                        })
                        .statements
                        .push(ferronconf::Statement::Directive(ferronconf::Directive {
                            name: "eab".to_string(),
                            args: vec![
                                ferronconf::Value::String(
                                    key_id,
                                    ferronconf::Span { line: 0, column: 0 },
                                ),
                                ferronconf::Value::String(
                                    hmac,
                                    ferronconf::Span { line: 0, column: 0 },
                                ),
                            ],
                            block: None,
                            span: ferronconf::Span { line: 0, column: 0 },
                        }));
                }
            }
            "auto_tls_save_data" => {
                let cert = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                let key = node.entries.get(1).and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                if let (Some(cert), Some(key)) = (cert, key) {
                    nested_directives
                        .entry("tls")
                        .or_insert_with(|| ferronconf::Block {
                            statements: vec![],
                            span: ferronconf::Span { line: 0, column: 0 },
                        })
                        .statements
                        .push(ferronconf::Statement::Directive(ferronconf::Directive {
                            name: "save".to_string(),
                            args: vec![
                                ferronconf::Value::String(
                                    cert,
                                    ferronconf::Span { line: 0, column: 0 },
                                ),
                                ferronconf::Value::String(
                                    key,
                                    ferronconf::Span { line: 0, column: 0 },
                                ),
                            ],
                            block: None,
                            span: ferronconf::Span { line: 0, column: 0 },
                        }));
                }
            }
            "auto_tls_post_obtain_command" => {
                let cmd = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                if let Some(cmd) = cmd {
                    nested_directives
                        .entry("tls")
                        .or_insert_with(|| ferronconf::Block {
                            statements: vec![],
                            span: ferronconf::Span { line: 0, column: 0 },
                        })
                        .statements
                        .push(ferronconf::Statement::Directive(ferronconf::Directive {
                            name: "post_obtain_command".to_string(),
                            args: vec![ferronconf::Value::String(
                                cmd,
                                ferronconf::Span { line: 0, column: 0 },
                            )],
                            block: None,
                            span: ferronconf::Span { line: 0, column: 0 },
                        }));
                }
            }
            "tls_cipher_suite" | "tls_ecdh_curve" | "tls_min_version" | "tls_max_version" => {
                let val = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                if let Some(val) = val {
                    let name = match node.name() {
                        "tls_cipher_suite" => "cipher_suite",
                        "tls_ecdh_curve" => "ecdh_curve",
                        "tls_min_version" => "min_version",
                        "tls_max_version" => "max_version",
                        _ => unreachable!(),
                    };
                    nested_directives
                        .entry("tls")
                        .or_insert_with(|| ferronconf::Block {
                            statements: vec![],
                            span: ferronconf::Span { line: 0, column: 0 },
                        })
                        .statements
                        .push(ferronconf::Statement::Directive(ferronconf::Directive {
                            name: name.to_string(),
                            args: vec![ferronconf::Value::String(
                                val,
                                ferronconf::Span { line: 0, column: 0 },
                            )],
                            block: None,
                            span: ferronconf::Span { line: 0, column: 0 },
                        }));
                }
            }
            "tls_client_certificate" => {
                let val = node.entries.first();
                if let Some(e) = val {
                    match &e.value {
                        kdlite::dom::Value::Bool(b) => {
                            nested_directives
                                .entry("tls")
                                .or_insert_with(|| ferronconf::Block {
                                    statements: vec![],
                                    span: ferronconf::Span { line: 0, column: 0 },
                                })
                                .statements
                                .push(ferronconf::Statement::Directive(ferronconf::Directive {
                                    name: "client_auth".to_string(),
                                    args: vec![ferronconf::Value::Boolean(
                                        *b,
                                        ferronconf::Span { line: 0, column: 0 },
                                    )],
                                    block: None,
                                    span: ferronconf::Span { line: 0, column: 0 },
                                }));
                        }
                        kdlite::dom::Value::String(s) => {
                            nested_directives
                                .entry("tls")
                                .or_insert_with(|| ferronconf::Block {
                                    statements: vec![],
                                    span: ferronconf::Span { line: 0, column: 0 },
                                })
                                .statements
                                .push(ferronconf::Statement::Directive(ferronconf::Directive {
                                    name: "client_auth_ca".to_string(),
                                    args: vec![ferronconf::Value::String(
                                        s.to_string(),
                                        ferronconf::Span { line: 0, column: 0 },
                                    )],
                                    block: None,
                                    span: ferronconf::Span { line: 0, column: 0 },
                                }));
                        }
                        _ => {}
                    }
                }
            }
            "ocsp_stapling" => {
                let enabled = node.entries.first().is_none_or(|e| match e.value {
                    kdlite::dom::Value::Bool(b) => b,
                    _ => true,
                });
                let mut ocsp_block = ferronconf::Block {
                    statements: vec![],
                    span: ferronconf::Span { line: 0, column: 0 },
                };
                ocsp_block.statements.push(ferronconf::Statement::Directive(
                    ferronconf::Directive {
                        name: "enabled".to_string(),
                        args: vec![ferronconf::Value::Boolean(
                            enabled,
                            ferronconf::Span { line: 0, column: 0 },
                        )],
                        block: None,
                        span: ferronconf::Span { line: 0, column: 0 },
                    },
                ));
                nested_directives
                    .entry("tls")
                    .or_insert_with(|| ferronconf::Block {
                        statements: vec![],
                        span: ferronconf::Span { line: 0, column: 0 },
                    })
                    .statements
                    .push(ferronconf::Statement::Directive(ferronconf::Directive {
                        name: "ocsp".to_string(),
                        args: vec![],
                        block: Some(ocsp_block),
                        span: ferronconf::Span { line: 0, column: 0 },
                    }));
            }
            "session_tickets" => {
                let enabled = node.entries.first().is_none_or(|e| match e.value {
                    kdlite::dom::Value::Bool(b) => b,
                    _ => true,
                });
                if enabled {
                    nested_directives
                        .entry("tls")
                        .or_insert_with(|| ferronconf::Block {
                            statements: vec![],
                            span: ferronconf::Span { line: 0, column: 0 },
                        })
                        .statements
                        .push(ferronconf::Statement::Directive(ferronconf::Directive {
                            name: "ticket_keys".to_string(),
                            args: vec![],
                            block: Some(ferronconf::Block {
                                statements: vec![],
                                span: ferronconf::Span { line: 0, column: 0 },
                            }),
                            span: ferronconf::Span { line: 0, column: 0 },
                        }));
                }
            }
            // Application Backends
            "cgi" => {
                let enabled = node.entries.first().is_none_or(|e| match e.value {
                    kdlite::dom::Value::Bool(b) => b,
                    _ => true,
                });
                if enabled {
                    let cgi_block = ferronconf::Block {
                        statements: vec![],
                        span: ferronconf::Span { line: 0, column: 0 },
                    };
                    statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                        name: "cgi".to_string(),
                        args: vec![],
                        block: Some(cgi_block),
                        span: ferronconf::Span { line: 0, column: 0 },
                    }));
                }
            }
            "cgi_extension" => {
                let extensions = node
                    .entries
                    .iter()
                    .filter_map(|e| match &e.value {
                        kdlite::dom::Value::String(s) => Some(s.to_string()),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                for ext in extensions {
                    nested_directives
                        .entry("cgi")
                        .or_insert_with(|| ferronconf::Block {
                            statements: vec![],
                            span: ferronconf::Span { line: 0, column: 0 },
                        })
                        .statements
                        .push(ferronconf::Statement::Directive(ferronconf::Directive {
                            name: "extension".to_string(),
                            args: vec![ferronconf::Value::String(
                                ext,
                                ferronconf::Span { line: 0, column: 0 },
                            )],
                            block: None,
                            span: ferronconf::Span { line: 0, column: 0 },
                        }));
                }
            }
            "cgi_interpreter" => {
                let args = node
                    .entries
                    .iter()
                    .map(|e| match &e.value {
                        kdlite::dom::Value::String(s) => ferronconf::Value::String(
                            s.to_string(),
                            ferronconf::Span { line: 0, column: 0 },
                        ),
                        _ => ferronconf::Value::Boolean(
                            false,
                            ferronconf::Span { line: 0, column: 0 },
                        ),
                    })
                    .collect::<Vec<_>>();
                nested_directives
                    .entry("cgi")
                    .or_insert_with(|| ferronconf::Block {
                        statements: vec![],
                        span: ferronconf::Span { line: 0, column: 0 },
                    })
                    .statements
                    .push(ferronconf::Statement::Directive(ferronconf::Directive {
                        name: "interpreter".to_string(),
                        args,
                        block: None,
                        span: ferronconf::Span { line: 0, column: 0 },
                    }));
            }
            "cgi_environment" => {
                let name = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                let value = node.entries.get(1).and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                if let (Some(name), Some(value)) = (name, value) {
                    nested_directives
                        .entry("cgi")
                        .or_insert_with(|| ferronconf::Block {
                            statements: vec![],
                            span: ferronconf::Span { line: 0, column: 0 },
                        })
                        .statements
                        .push(ferronconf::Statement::Directive(ferronconf::Directive {
                            name: "environment".to_string(),
                            args: vec![
                                ferronconf::Value::String(
                                    name,
                                    ferronconf::Span { line: 0, column: 0 },
                                ),
                                ferronconf::Value::String(
                                    value,
                                    ferronconf::Span { line: 0, column: 0 },
                                ),
                            ],
                            block: None,
                            span: ferronconf::Span { line: 0, column: 0 },
                        }));
                }
            }
            "scgi" => {
                let to = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                if let Some(to) = to {
                    statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                        name: "scgi".to_string(),
                        args: vec![ferronconf::Value::String(
                            to,
                            ferronconf::Span { line: 0, column: 0 },
                        )],
                        block: None,
                        span: ferronconf::Span { line: 0, column: 0 },
                    }));
                }
            }
            "scgi_environment" => {
                let name = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                let value = node.entries.get(1).and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                if let (Some(name), Some(value)) = (name, value) {
                    nested_directives
                        .entry("scgi")
                        .or_insert_with(|| ferronconf::Block {
                            statements: vec![],
                            span: ferronconf::Span { line: 0, column: 0 },
                        })
                        .statements
                        .push(ferronconf::Statement::Directive(ferronconf::Directive {
                            name: "environment".to_string(),
                            args: vec![
                                ferronconf::Value::String(
                                    name,
                                    ferronconf::Span { line: 0, column: 0 },
                                ),
                                ferronconf::Value::String(
                                    value,
                                    ferronconf::Span { line: 0, column: 0 },
                                ),
                            ],
                            block: None,
                            span: ferronconf::Span { line: 0, column: 0 },
                        }));
                }
            }
            "fcgi" => {
                let to = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                let pass = node
                    .entries
                    .iter()
                    .find(|e| e.key() == Some("pass"))
                    .is_none_or(|e| match &e.value {
                        kdlite::dom::Value::Bool(b) => *b,
                        _ => true,
                    });

                let mut fcgi_block = ferronconf::Block {
                    statements: vec![],
                    span: ferronconf::Span { line: 0, column: 0 },
                };
                if let Some(to) = to {
                    fcgi_block.statements.push(ferronconf::Statement::Directive(
                        ferronconf::Directive {
                            name: "backend".to_string(),
                            args: vec![ferronconf::Value::String(
                                to,
                                ferronconf::Span { line: 0, column: 0 },
                            )],
                            block: None,
                            span: ferronconf::Span { line: 0, column: 0 },
                        },
                    ));
                }
                fcgi_block.statements.push(ferronconf::Statement::Directive(
                    ferronconf::Directive {
                        name: "pass".to_string(),
                        args: vec![ferronconf::Value::Boolean(
                            pass,
                            ferronconf::Span { line: 0, column: 0 },
                        )],
                        block: None,
                        span: ferronconf::Span { line: 0, column: 0 },
                    },
                ));

                statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                    name: "fcgi".to_string(),
                    args: vec![],
                    block: Some(fcgi_block),
                    span: ferronconf::Span { line: 0, column: 0 },
                }));
            }
            "fcgi_php" => {
                let to = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                if let Some(to) = to {
                    statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                        name: "fcgi_php".to_string(),
                        args: vec![ferronconf::Value::String(
                            to,
                            ferronconf::Span { line: 0, column: 0 },
                        )],
                        block: None,
                        span: ferronconf::Span { line: 0, column: 0 },
                    }));
                }
            }
            "fcgi_extension" => {
                let extensions = node
                    .entries
                    .iter()
                    .filter_map(|e| match &e.value {
                        kdlite::dom::Value::String(s) => Some(s.to_string()),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                for ext in extensions {
                    nested_directives
                        .entry("fcgi")
                        .or_insert_with(|| ferronconf::Block {
                            statements: vec![],
                            span: ferronconf::Span { line: 0, column: 0 },
                        })
                        .statements
                        .push(ferronconf::Statement::Directive(ferronconf::Directive {
                            name: "extension".to_string(),
                            args: vec![ferronconf::Value::String(
                                ext,
                                ferronconf::Span { line: 0, column: 0 },
                            )],
                            block: None,
                            span: ferronconf::Span { line: 0, column: 0 },
                        }));
                }
            }
            "fcgi_environment" => {
                let name = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                let value = node.entries.get(1).and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                if let (Some(name), Some(value)) = (name, value) {
                    nested_directives
                        .entry("fcgi")
                        .or_insert_with(|| ferronconf::Block {
                            statements: vec![],
                            span: ferronconf::Span { line: 0, column: 0 },
                        })
                        .statements
                        .push(ferronconf::Statement::Directive(ferronconf::Directive {
                            name: "environment".to_string(),
                            args: vec![
                                ferronconf::Value::String(
                                    name,
                                    ferronconf::Span { line: 0, column: 0 },
                                ),
                                ferronconf::Value::String(
                                    value,
                                    ferronconf::Span { line: 0, column: 0 },
                                ),
                            ],
                            block: None,
                            span: ferronconf::Span { line: 0, column: 0 },
                        }));
                }
            }
            "trust_x_forwarded_for" => {
                let enabled = node.entries.first().is_none_or(|e| match e.value {
                    kdlite::dom::Value::Bool(b) => b,
                    _ => true,
                });
                if enabled {
                    statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                        name: "client_ip_from_header".to_string(),
                        args: vec![ferronconf::Value::String(
                            "x-forwarded-for".to_string(),
                            ferronconf::Span { line: 0, column: 0 },
                        )],
                        block: Some(ferronconf::Block {
                            statements: vec![ferronconf::Statement::Directive(
                                ferronconf::Directive {
                                    name: "trusted_proxy".to_string(),
                                    args: vec![ferronconf::Value::String(
                                        "0.0.0.0/0".to_string(),
                                        ferronconf::Span { line: 0, column: 0 },
                                    )],
                                    block: None,
                                    span: ferronconf::Span { line: 0, column: 0 },
                                },
                            )],
                            span: ferronconf::Span { line: 0, column: 0 },
                        }),
                        span: ferronconf::Span { line: 0, column: 0 },
                    }));
                }
            }
            "status" => {
                let code = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::Integer(i) => Some(*i as i64),
                    _ => None,
                });
                let mut status_block = ferronconf::Block {
                    statements: vec![],
                    span: ferronconf::Span { line: 0, column: 0 },
                };
                for entry in &node.entries[1..] {
                    if let Some(key) = entry.key() {
                        match key {
                            "body" => {
                                if let kdlite::dom::Value::String(s) = &entry.value {
                                    status_block
                                        .statements
                                        .push(ferronconf::Statement::Directive(
                                            ferronconf::Directive {
                                                name: "body".to_string(),
                                                args: vec![ferronconf::Value::String(
                                                    s.to_string(),
                                                    ferronconf::Span { line: 0, column: 0 },
                                                )],
                                                block: None,
                                                span: ferronconf::Span { line: 0, column: 0 },
                                            },
                                        ));
                                }
                            }
                            "location" => {
                                if let kdlite::dom::Value::String(s) = &entry.value {
                                    status_block
                                        .statements
                                        .push(ferronconf::Statement::Directive(
                                            ferronconf::Directive {
                                                name: "location".to_string(),
                                                args: vec![
                                                    convert_placeholders_into_interpolated_strings(
                                                        s,
                                                    ),
                                                ],
                                                block: None,
                                                span: ferronconf::Span { line: 0, column: 0 },
                                            },
                                        ));
                                }
                            }
                            _ => {}
                        }
                    }
                }
                statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                    name: "status".to_string(),
                    args: if let Some(code) = code {
                        vec![ferronconf::Value::Integer(
                            code,
                            ferronconf::Span { line: 0, column: 0 },
                        )]
                    } else {
                        vec![]
                    },
                    block: Some(status_block),
                    span: ferronconf::Span { line: 0, column: 0 },
                }));
            }
            "user" => {
                let name = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                let hash = node.entries.get(1).and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                if let (Some(name), Some(hash)) = (name, hash) {
                    nested_directives
                        .entry("basic_auth")
                        .or_insert_with(|| ferronconf::Block {
                            statements: vec![],
                            span: ferronconf::Span { line: 0, column: 0 },
                        })
                        .statements
                        .push(ferronconf::Statement::Directive(ferronconf::Directive {
                            name: "users".to_string(),
                            args: vec![],
                            block: Some(ferronconf::Block {
                                statements: vec![ferronconf::Statement::Directive(
                                    ferronconf::Directive {
                                        name,
                                        args: vec![ferronconf::Value::String(
                                            hash,
                                            ferronconf::Span { line: 0, column: 0 },
                                        )],
                                        block: None,
                                        span: ferronconf::Span { line: 0, column: 0 },
                                    },
                                )],
                                span: ferronconf::Span { line: 0, column: 0 },
                            }),
                            span: ferronconf::Span { line: 0, column: 0 },
                        }));
                }
            }
            "block" => {
                let ips = node
                    .entries
                    .iter()
                    .filter_map(|e| match &e.value {
                        kdlite::dom::Value::String(s) => Some(s.to_string()),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                for ip in ips {
                    statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                        name: "block".to_string(),
                        args: vec![ferronconf::Value::String(
                            ip,
                            ferronconf::Span { line: 0, column: 0 },
                        )],
                        block: None,
                        span: ferronconf::Span { line: 0, column: 0 },
                    }));
                }
            }
            "allow" => {
                let ips = node
                    .entries
                    .iter()
                    .filter_map(|e| match &e.value {
                        kdlite::dom::Value::String(s) => Some(s.to_string()),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                for ip in ips {
                    statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                        name: "allow".to_string(),
                        args: vec![ferronconf::Value::String(
                            ip,
                            ferronconf::Span { line: 0, column: 0 },
                        )],
                        block: None,
                        span: ferronconf::Span { line: 0, column: 0 },
                    }));
                }
            }
            "abort" => {
                let enabled = node.entries.first().is_none_or(|e| match e.value {
                    kdlite::dom::Value::Bool(b) => b,
                    _ => true,
                });
                statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                    name: "abort".to_string(),
                    args: vec![ferronconf::Value::Boolean(
                        enabled,
                        ferronconf::Span { line: 0, column: 0 },
                    )],
                    block: None,
                    span: ferronconf::Span { line: 0, column: 0 },
                }));
            }
            "limit" => {
                let enabled = node.entries.first().is_none_or(|e| match e.value {
                    kdlite::dom::Value::Bool(b) => b,
                    _ => true,
                });
                if !enabled {
                    statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                        name: "rate_limit".to_string(),
                        args: vec![ferronconf::Value::Boolean(
                            false,
                            ferronconf::Span { line: 0, column: 0 },
                        )],
                        block: None,
                        span: ferronconf::Span { line: 0, column: 0 },
                    }));
                } else {
                    let rate: Option<f64> = node
                        .entries
                        .iter()
                        .find(|e| e.key() == Some("rate"))
                        .and_then(|e| match &e.value {
                            kdlite::dom::Value::Integer(i) => Some(*i as f64),
                            kdlite::dom::Value::Float(i) => Some(*i),
                            _ => None,
                        });
                    let burst: Option<f64> = node
                        .entries
                        .iter()
                        .find(|e| e.key() == Some("burst"))
                        .and_then(|e| match &e.value {
                            kdlite::dom::Value::Integer(i) => Some(*i as f64),
                            kdlite::dom::Value::Float(i) => Some(*i),
                            _ => None,
                        });

                    let mut rl_block = ferronconf::Block {
                        statements: vec![],
                        span: ferronconf::Span { line: 0, column: 0 },
                    };
                    if let Some(rate) = rate {
                        rl_block.statements.push(ferronconf::Statement::Directive(
                            ferronconf::Directive {
                                name: "rate".to_string(),
                                args: vec![ferronconf::Value::Float(
                                    rate,
                                    ferronconf::Span { line: 0, column: 0 },
                                )],
                                block: None,
                                span: ferronconf::Span { line: 0, column: 0 },
                            },
                        ));
                    }
                    if let Some(burst) = burst {
                        rl_block.statements.push(ferronconf::Statement::Directive(
                            ferronconf::Directive {
                                name: "burst".to_string(),
                                args: vec![ferronconf::Value::Float(
                                    burst,
                                    ferronconf::Span { line: 0, column: 0 },
                                )],
                                block: None,
                                span: ferronconf::Span { line: 0, column: 0 },
                            },
                        ));
                    }
                    statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                        name: "rate_limit".to_string(),
                        args: vec![],
                        block: Some(rl_block),
                        span: ferronconf::Span { line: 0, column: 0 },
                    }));
                }
            }
            "log" => {
                let path = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                if let Some(path) = path {
                    let mut log_block = ferronconf::Block {
                        statements: vec![],
                        span: ferronconf::Span { line: 0, column: 0 },
                    };
                    log_block.statements.push(ferronconf::Statement::Directive(
                        ferronconf::Directive {
                            name: "access_log".to_string(),
                            args: vec![ferronconf::Value::String(
                                path,
                                ferronconf::Span { line: 0, column: 0 },
                            )],
                            block: None,
                            span: ferronconf::Span { line: 0, column: 0 },
                        },
                    ));
                    nested_directives
                        .entry("observability")
                        .or_insert_with(|| ferronconf::Block {
                            statements: vec![],
                            span: ferronconf::Span { line: 0, column: 0 },
                        })
                        .statements
                        .push(ferronconf::Statement::Directive(ferronconf::Directive {
                            name: "provider".to_string(),
                            args: vec![ferronconf::Value::String(
                                "file".to_string(),
                                ferronconf::Span { line: 0, column: 0 },
                            )],
                            block: Some(log_block),
                            span: ferronconf::Span { line: 0, column: 0 },
                        }));
                }
            }
            "error_log" => {
                let path = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                if let Some(path) = path {
                    nested_directives
                        .entry("observability")
                        .or_insert_with(|| ferronconf::Block {
                            statements: vec![],
                            span: ferronconf::Span { line: 0, column: 0 },
                        })
                        .statements
                        .push(ferronconf::Statement::Directive(ferronconf::Directive {
                            name: "provider".to_string(),
                            args: vec![ferronconf::Value::String(
                                "file".to_string(),
                                ferronconf::Span { line: 0, column: 0 },
                            )],
                            block: Some(ferronconf::Block {
                                statements: vec![ferronconf::Statement::Directive(
                                    ferronconf::Directive {
                                        name: "error_log".to_string(),
                                        args: vec![ferronconf::Value::String(
                                            path,
                                            ferronconf::Span { line: 0, column: 0 },
                                        )],
                                        block: None,
                                        span: ferronconf::Span { line: 0, column: 0 },
                                    },
                                )],
                                span: ferronconf::Span { line: 0, column: 0 },
                            }),
                            span: ferronconf::Span { line: 0, column: 0 },
                        }));
                }
            }
            "otlp_logs" | "otlp_metrics" | "otlp_traces" => {
                let endpoint = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                if let Some(endpoint) = endpoint {
                    let mut signal_block = ferronconf::Block {
                        statements: vec![],
                        span: ferronconf::Span { line: 0, column: 0 },
                    };
                    if let Some(proto) = node
                        .entries
                        .iter()
                        .find(|e| e.key() == Some("protocol"))
                        .and_then(|e| match &e.value {
                            kdlite::dom::Value::String(s) => Some(s.to_string()),
                            _ => None,
                        })
                    {
                        signal_block
                            .statements
                            .push(ferronconf::Statement::Directive(ferronconf::Directive {
                                name: "protocol".to_string(),
                                args: vec![ferronconf::Value::String(
                                    proto,
                                    ferronconf::Span { line: 0, column: 0 },
                                )],
                                block: None,
                                span: ferronconf::Span { line: 0, column: 0 },
                            }));
                    }
                    if let Some(auth) = node
                        .entries
                        .iter()
                        .find(|e| e.key() == Some("authorization"))
                        .and_then(|e| match &e.value {
                            kdlite::dom::Value::String(s) => Some(s.to_string()),
                            _ => None,
                        })
                    {
                        signal_block
                            .statements
                            .push(ferronconf::Statement::Directive(ferronconf::Directive {
                                name: "authorization".to_string(),
                                args: vec![ferronconf::Value::String(
                                    auth,
                                    ferronconf::Span { line: 0, column: 0 },
                                )],
                                block: None,
                                span: ferronconf::Span { line: 0, column: 0 },
                            }));
                    }
                    nested_directives
                        .entry("observability")
                        .or_insert_with(|| ferronconf::Block {
                            statements: vec![],
                            span: ferronconf::Span { line: 0, column: 0 },
                        })
                        .statements
                        .push(ferronconf::Statement::Directive(ferronconf::Directive {
                            name: "provider".to_string(),
                            args: vec![ferronconf::Value::String(
                                "otlp".to_string(),
                                ferronconf::Span { line: 0, column: 0 },
                            )],
                            block: Some(ferronconf::Block {
                                statements: vec![ferronconf::Statement::Directive(
                                    ferronconf::Directive {
                                        name: node.name().replace("otlp_", ""),
                                        args: vec![ferronconf::Value::String(
                                            endpoint,
                                            ferronconf::Span { line: 0, column: 0 },
                                        )],
                                        block: Some(signal_block),
                                        span: ferronconf::Span { line: 0, column: 0 },
                                    },
                                )],
                                span: ferronconf::Span { line: 0, column: 0 },
                            }),
                            span: ferronconf::Span { line: 0, column: 0 },
                        }));
                }
            }
            "log_date_format" => {
                let val = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                if let Some(val) = val {
                    let statements = &mut nested_directives
                        .entry("observability")
                        .or_insert_with(|| ferronconf::Block {
                            statements: vec![],
                            span: ferronconf::Span { line: 0, column: 0 },
                        })
                        .statements;
                    statements.iter_mut().for_each(|s| {
                        if let ferronconf::Statement::Directive(d) = s {
                            if d.name == "access_pattern" {
                                d.args.iter_mut().for_each(|a| {
                                    if let ferronconf::Value::String(s, _) = a {
                                        *s = s.replace("%t", &format!("%{{{val}}}t"));
                                    }
                                });
                            }
                        }
                    });
                    date_format = Some(val);
                }
            }
            "log_format" => {
                let val = node
                    .entries
                    .first()
                    .and_then(|e| match &e.value {
                        kdlite::dom::Value::String(s) => Some(s.to_string()),
                        _ => None,
                    })
                    .map(|mut val| {
                        while let Some(start) = val.find('{') {
                            if let Some(end) = val[start + 1..].find('}') {
                                let converted = match &val[start + 1..end] {
                                    "path" => "%path".to_string(),
                                    "path_and_query" => "%path_and_query".to_string(),
                                    "method" => "%method".to_string(),
                                    "version" => "%version".to_string(),
                                    "scheme" => "%scheme".to_string(),
                                    "client_ip" => "%client_ip".to_string(),
                                    "client_port" => "%client_port".to_string(),
                                    "client_ip_canonical" => "%client_ip_canonical".to_string(),
                                    "server_ip" => "%server_ip".to_string(),
                                    "server_port" => "%server_port".to_string(),
                                    "server_ip_canonical" => "%server_ip_canonical".to_string(),
                                    "auth_user" => "%auth_user".to_string(),
                                    "status_code" => "%status".to_string(),
                                    "timestamp" => {
                                        if let Some(date_format) = &date_format {
                                            format!("%{{{}}}t", date_format)
                                        } else {
                                            "%t".to_string()
                                        }
                                    }
                                    "content_length" => "%content_length".to_string(),
                                    s if s.starts_with("header:") => {
                                        format!("%{{{}}}i", &s[7..])
                                    }
                                    _ => "".to_string(), // Replace with nothing
                                };
                                val.replace_range(start..=end, &converted);
                            } else {
                                break;
                            }
                        }
                        val
                    });
                if let Some(val) = val {
                    nested_directives
                        .entry("observability")
                        .or_insert_with(|| ferronconf::Block {
                            statements: vec![],
                            span: ferronconf::Span { line: 0, column: 0 },
                        })
                        .statements
                        .push(ferronconf::Statement::Directive(ferronconf::Directive {
                            name: "format".to_string(),
                            args: vec![ferronconf::Value::String(
                                val,
                                ferronconf::Span { line: 0, column: 0 },
                            )],
                            block: None,
                            span: ferronconf::Span { line: 0, column: 0 },
                        }));
                }
            }
            "log_json" => {
                let mut log_block = ferronconf::Block {
                    statements: vec![],
                    span: ferronconf::Span { line: 0, column: 0 },
                };
                log_block.statements.push(ferronconf::Statement::Directive(
                    ferronconf::Directive {
                        name: "format".to_string(),
                        args: vec![ferronconf::Value::String(
                            "json".to_string(),
                            ferronconf::Span { line: 0, column: 0 },
                        )],
                        block: None,
                        span: ferronconf::Span { line: 0, column: 0 },
                    },
                ));
                for entry in &node.entries {
                    if let Some(key) = entry.key() {
                        if let kdlite::dom::Value::String(s) = &entry.value {
                            log_block.statements.push(ferronconf::Statement::Directive(
                                ferronconf::Directive {
                                    name: "field".to_string(),
                                    args: vec![
                                        ferronconf::Value::String(
                                            key.to_string(),
                                            ferronconf::Span { line: 0, column: 0 },
                                        ),
                                        ferronconf::Value::String(
                                            s.to_string(),
                                            ferronconf::Span { line: 0, column: 0 },
                                        ),
                                    ],
                                    block: None,
                                    span: ferronconf::Span { line: 0, column: 0 },
                                },
                            ));
                        }
                    }
                }
                nested_directives
                    .entry("observability")
                    .or_insert_with(|| ferronconf::Block {
                        statements: vec![],
                        span: ferronconf::Span { line: 0, column: 0 },
                    })
                    .statements
                    .push(ferronconf::Statement::Directive(ferronconf::Directive {
                        name: "provider".to_string(),
                        args: vec![ferronconf::Value::String(
                            "file".to_string(),
                            ferronconf::Span { line: 0, column: 0 },
                        )],
                        block: Some(log_block),
                        span: ferronconf::Span { line: 0, column: 0 },
                    }));
            }
            "otlp_service_name" | "otlp_no_verification" => {
                let name = match node.name() {
                    "otlp_service_name" => "service_name",
                    "otlp_no_verification" => "no_verify",
                    _ => unreachable!(),
                };
                let val = node.entries.first().map(|e| match &e.value {
                    kdlite::dom::Value::String(s) => ferronconf::Value::String(
                        s.to_string(),
                        ferronconf::Span { line: 0, column: 0 },
                    ),
                    kdlite::dom::Value::Bool(b) => {
                        ferronconf::Value::Boolean(*b, ferronconf::Span { line: 0, column: 0 })
                    }
                    _ => ferronconf::Value::Boolean(true, ferronconf::Span { line: 0, column: 0 }),
                });
                if let Some(val) = val {
                    nested_directives
                        .entry("observability")
                        .or_insert_with(|| ferronconf::Block {
                            statements: vec![],
                            span: ferronconf::Span { line: 0, column: 0 },
                        })
                        .statements
                        .push(ferronconf::Statement::Directive(ferronconf::Directive {
                            name: name.to_string(),
                            args: vec![val],
                            block: None,
                            span: ferronconf::Span { line: 0, column: 0 },
                        }));
                }
            }
            "log_rotate_size"
            | "log_rotate_keep"
            | "error_log_rotate_size"
            | "error_log_rotate_keep" => {
                let val = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::Integer(i) => Some(*i as i64),
                    _ => None,
                });
                if let Some(val) = val {
                    let name = if let Some(n) = node.name().strip_prefix("error_log_") {
                        format!("error_log_{n}")
                    } else if let Some(n) = node.name().strip_prefix("log_") {
                        format!("access_log_{n}")
                    } else {
                        node.name().to_owned()
                    };
                    nested_directives
                        .entry("observability")
                        .or_insert_with(|| ferronconf::Block {
                            statements: vec![],
                            span: ferronconf::Span { line: 0, column: 0 },
                        })
                        .statements
                        .push(ferronconf::Statement::Directive(ferronconf::Directive {
                            name,
                            args: vec![ferronconf::Value::Integer(
                                val,
                                ferronconf::Span { line: 0, column: 0 },
                            )],
                            block: None,
                            span: ferronconf::Span { line: 0, column: 0 },
                        }));
                }
            }

            // Reverse Proxying

            // Reverse Proxying
            "proxy" => {
                let to = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                let proxy_block =
                    nested_directives
                        .entry("proxy")
                        .or_insert_with(|| ferronconf::Block {
                            statements: vec![],
                            span: ferronconf::Span { line: 0, column: 0 },
                        });
                if let Some(to) = to {
                    proxy_block
                        .statements
                        .push(ferronconf::Statement::Directive(ferronconf::Directive {
                            name: "upstream".to_string(),
                            args: vec![ferronconf::Value::String(
                                to,
                                ferronconf::Span { line: 0, column: 0 },
                            )],
                            block: None,
                            span: ferronconf::Span { line: 0, column: 0 },
                        }));
                }
                for entry in &node.entries[1..] {
                    if let Some(key) = entry.key() {
                        match key {
                            "unix" => {
                                if let kdlite::dom::Value::String(s) = &entry.value {
                                    proxy_block
                                        .statements
                                        .push(ferronconf::Statement::Directive(
                                            ferronconf::Directive {
                                                name: "upstream".to_string(),
                                                args: vec![ferronconf::Value::String(
                                                    "http://backend".to_string(),
                                                    ferronconf::Span { line: 0, column: 0 },
                                                )],
                                                block: Some(ferronconf::Block {
                                                    statements: vec![
                                                        ferronconf::Statement::Directive(
                                                            ferronconf::Directive {
                                                                name: "unix".to_string(),
                                                                args: vec![
                                                                    ferronconf::Value::String(
                                                                        s.to_string(),
                                                                        ferronconf::Span {
                                                                            line: 0,
                                                                            column: 0,
                                                                        },
                                                                    ),
                                                                ],
                                                                block: None,
                                                                span: ferronconf::Span {
                                                                    line: 0,
                                                                    column: 0,
                                                                },
                                                            },
                                                        ),
                                                    ],
                                                    span: ferronconf::Span { line: 0, column: 0 },
                                                }),
                                                span: ferronconf::Span { line: 0, column: 0 },
                                            },
                                        ));
                                }
                            }
                            "limit" => {
                                if let kdlite::dom::Value::Integer(i) = &entry.value {
                                    proxy_block
                                        .statements
                                        .push(ferronconf::Statement::Directive(
                                            ferronconf::Directive {
                                                name: "limit".to_string(),
                                                args: vec![ferronconf::Value::Integer(
                                                    *i as i64,
                                                    ferronconf::Span { line: 0, column: 0 },
                                                )],
                                                block: None,
                                                span: ferronconf::Span { line: 0, column: 0 },
                                            },
                                        ));
                                }
                            }
                            "idle_timeout" => {
                                if let kdlite::dom::Value::Integer(i) = &entry.value {
                                    proxy_block
                                        .statements
                                        .push(ferronconf::Statement::Directive(
                                            ferronconf::Directive {
                                                name: "idle_timeout".to_string(),
                                                args: vec![ferronconf::Value::String(
                                                    format!("{}ms", i),
                                                    ferronconf::Span { line: 0, column: 0 },
                                                )],
                                                block: None,
                                                span: ferronconf::Span { line: 0, column: 0 },
                                            },
                                        ));
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            "lb_health_check" => {
                let enabled = node.entries.first().is_none_or(|e| match e.value {
                    kdlite::dom::Value::Bool(b) => b,
                    _ => true,
                });
                nested_directives
                    .entry("proxy")
                    .or_insert_with(|| ferronconf::Block {
                        statements: vec![],
                        span: ferronconf::Span { line: 0, column: 0 },
                    })
                    .statements
                    .push(ferronconf::Statement::Directive(ferronconf::Directive {
                        name: "lb_health_check".to_string(),
                        args: vec![ferronconf::Value::Boolean(
                            enabled,
                            ferronconf::Span { line: 0, column: 0 },
                        )],
                        block: None,
                        span: ferronconf::Span { line: 0, column: 0 },
                    }));
            }
            "lb_health_check_max_fails" => {
                if let Some(e) = node.entries.first() {
                    if let kdlite::dom::Value::Integer(i) = &e.value {
                        nested_directives
                            .entry("proxy")
                            .or_insert_with(|| ferronconf::Block {
                                statements: vec![],
                                span: ferronconf::Span { line: 0, column: 0 },
                            })
                            .statements
                            .push(ferronconf::Statement::Directive(ferronconf::Directive {
                                name: "lb_health_check_max_fails".to_string(),
                                args: vec![ferronconf::Value::Integer(
                                    *i as i64,
                                    ferronconf::Span { line: 0, column: 0 },
                                )],
                                block: None,
                                span: ferronconf::Span { line: 0, column: 0 },
                            }));
                    }
                }
            }
            "proxy_no_verification"
            | "proxy_intercept_errors"
            | "proxy_keepalive"
            | "proxy_http2"
            | "lb_retry_connection"
            | "proxy_http2_only" => {
                let enabled = node.entries.first().is_none_or(|e| match e.value {
                    kdlite::dom::Value::Bool(b) => b,
                    _ => true,
                });
                nested_directives
                    .entry("proxy")
                    .or_insert_with(|| ferronconf::Block {
                        statements: vec![],
                        span: ferronconf::Span { line: 0, column: 0 },
                    })
                    .statements
                    .push(ferronconf::Statement::Directive(ferronconf::Directive {
                        name: node.name().to_string(),
                        args: vec![ferronconf::Value::Boolean(
                            enabled,
                            ferronconf::Span { line: 0, column: 0 },
                        )],
                        block: None,
                        span: ferronconf::Span { line: 0, column: 0 },
                    }));
            }
            "lb_algorithm" => {
                let alg = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                if let Some(alg) = alg {
                    nested_directives
                        .entry("proxy")
                        .or_insert_with(|| ferronconf::Block {
                            statements: vec![],
                            span: ferronconf::Span { line: 0, column: 0 },
                        })
                        .statements
                        .push(ferronconf::Statement::Directive(ferronconf::Directive {
                            name: "lb_algorithm".to_string(),
                            args: vec![ferronconf::Value::String(
                                alg,
                                ferronconf::Span { line: 0, column: 0 },
                            )],
                            block: None,
                            span: ferronconf::Span { line: 0, column: 0 },
                        }));
                }
            }
            "lb_health_check_window" => {
                if let Some(e) = node.entries.first() {
                    if let kdlite::dom::Value::Integer(i) = &e.value {
                        nested_directives
                            .entry("proxy")
                            .or_insert_with(|| ferronconf::Block {
                                statements: vec![],
                                span: ferronconf::Span { line: 0, column: 0 },
                            })
                            .statements
                            .push(ferronconf::Statement::Directive(ferronconf::Directive {
                                name: "lb_health_check_window".to_string(),
                                args: vec![ferronconf::Value::String(
                                    format!("{}ms", i),
                                    ferronconf::Span { line: 0, column: 0 },
                                )],
                                block: None,
                                span: ferronconf::Span { line: 0, column: 0 },
                            }));
                    }
                }
            }
            "proxy_proxy_header" => {
                let ver = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                if let Some(ver) = ver {
                    nested_directives
                        .entry("proxy")
                        .or_insert_with(|| ferronconf::Block {
                            statements: vec![],
                            span: ferronconf::Span { line: 0, column: 0 },
                        })
                        .statements
                        .push(ferronconf::Statement::Directive(ferronconf::Directive {
                            name: "proxy_header".to_string(),
                            args: vec![ferronconf::Value::String(
                                ver,
                                ferronconf::Span { line: 0, column: 0 },
                            )],
                            block: None,
                            span: ferronconf::Span { line: 0, column: 0 },
                        }));
                }
            }
            "proxy_srv" => {
                let to = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                if let Some(to) = to {
                    statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                        name: "proxy".to_string(),
                        args: vec![],
                        block: Some(ferronconf::Block {
                            statements: vec![ferronconf::Statement::Directive(
                                ferronconf::Directive {
                                    name: "srv".to_string(),
                                    args: vec![ferronconf::Value::String(
                                        to,
                                        ferronconf::Span { line: 0, column: 0 },
                                    )],
                                    block: None,
                                    span: ferronconf::Span { line: 0, column: 0 },
                                },
                            )],
                            span: ferronconf::Span { line: 0, column: 0 },
                        }),
                        span: ferronconf::Span { line: 0, column: 0 },
                    }));
                }
            }
            "forward_proxy" => {
                let enabled = node.entries.first().is_none_or(|e| match e.value {
                    kdlite::dom::Value::Bool(b) => b,
                    _ => true,
                });
                let mut fp_block = ferronconf::Block {
                    statements: vec![],
                    span: ferronconf::Span { line: 0, column: 0 },
                };
                if let Some(children) = &node.children {
                    for child in &children.nodes {
                        match child.name() {
                            "allow_domains" => {
                                let domains = child
                                    .entries
                                    .iter()
                                    .filter_map(|e| match &e.value {
                                        kdlite::dom::Value::String(s) => {
                                            Some(ferronconf::Value::String(
                                                s.to_string(),
                                                ferronconf::Span { line: 0, column: 0 },
                                            ))
                                        }
                                        _ => None,
                                    })
                                    .collect::<Vec<_>>();
                                fp_block.statements.push(ferronconf::Statement::Directive(
                                    ferronconf::Directive {
                                        name: "allow_domains".to_string(),
                                        args: domains,
                                        block: None,
                                        span: ferronconf::Span { line: 0, column: 0 },
                                    },
                                ));
                            }
                            "allow_ports" => {
                                let ports = child
                                    .entries
                                    .iter()
                                    .filter_map(|e| match &e.value {
                                        kdlite::dom::Value::Integer(i) => {
                                            Some(ferronconf::Value::Integer(
                                                *i as i64,
                                                ferronconf::Span { line: 0, column: 0 },
                                            ))
                                        }
                                        _ => None,
                                    })
                                    .collect::<Vec<_>>();
                                fp_block.statements.push(ferronconf::Statement::Directive(
                                    ferronconf::Directive {
                                        name: "allow_ports".to_string(),
                                        args: ports,
                                        block: None,
                                        span: ferronconf::Span { line: 0, column: 0 },
                                    },
                                ));
                            }
                            _ => {}
                        }
                    }
                }
                statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                    name: "forward_proxy".to_string(),
                    args: vec![],
                    block: if enabled { Some(fp_block) } else { None },
                    span: ferronconf::Span { line: 0, column: 0 },
                }));
            }
            "auth_to" => {
                let to = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                if let Some(to) = to {
                    statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                        name: "auth_to".to_string(),
                        args: vec![ferronconf::Value::String(
                            to,
                            ferronconf::Span { line: 0, column: 0 },
                        )],
                        block: None,
                        span: ferronconf::Span { line: 0, column: 0 },
                    }));
                }
            }
            "auth_to_no_verification" => {
                let enabled = node.entries.first().is_none_or(|e| match e.value {
                    kdlite::dom::Value::Bool(b) => b,
                    _ => true,
                });
                statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                    name: "auth_to".to_string(),
                    args: vec![],
                    block: Some(ferronconf::Block {
                        statements: vec![ferronconf::Statement::Directive(ferronconf::Directive {
                            name: "no_verification".to_string(),
                            args: vec![ferronconf::Value::Boolean(
                                enabled,
                                ferronconf::Span { line: 0, column: 0 },
                            )],
                            block: None,
                            span: ferronconf::Span { line: 0, column: 0 },
                        })],
                        span: ferronconf::Span { line: 0, column: 0 },
                    }),
                    span: ferronconf::Span { line: 0, column: 0 },
                }));
            }
            "auth_to_copy" => {
                let headers = node
                    .entries
                    .iter()
                    .filter_map(|e| match &e.value {
                        kdlite::dom::Value::String(s) => Some(ferronconf::Value::String(
                            s.to_string(),
                            ferronconf::Span { line: 0, column: 0 },
                        )),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                    name: "auth_to".to_string(),
                    args: vec![],
                    block: Some(ferronconf::Block {
                        statements: vec![ferronconf::Statement::Directive(ferronconf::Directive {
                            name: "copy".to_string(),
                            args: headers,
                            block: None,
                            span: ferronconf::Span { line: 0, column: 0 },
                        })],
                        span: ferronconf::Span { line: 0, column: 0 },
                    }),
                    span: ferronconf::Span { line: 0, column: 0 },
                }));
            }
            "proxy_request_header"
            | "proxy_request_header_remove"
            | "proxy_request_header_replace" => {
                let name = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                let value = node.entries.get(1).and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => {
                        Some(convert_placeholders_into_interpolated_strings(s))
                    }
                    _ => None,
                });

                let directive = match node.name() {
                    "proxy_request_header" => {
                        if let (Some(name), Some(value)) = (name, value) {
                            ferronconf::Statement::Directive(ferronconf::Directive {
                                name: "request_header".to_string(),
                                args: vec![
                                    ferronconf::Value::String(
                                        format!("+{}", name),
                                        ferronconf::Span { line: 0, column: 0 },
                                    ),
                                    value,
                                ],
                                block: None,
                                span: ferronconf::Span { line: 0, column: 0 },
                            })
                        } else {
                            continue;
                        }
                    }
                    "proxy_request_header_remove" => {
                        if let Some(name) = name {
                            ferronconf::Statement::Directive(ferronconf::Directive {
                                name: "request_header".to_string(),
                                args: vec![ferronconf::Value::String(
                                    format!("-{}", name),
                                    ferronconf::Span { line: 0, column: 0 },
                                )],
                                block: None,
                                span: ferronconf::Span { line: 0, column: 0 },
                            })
                        } else {
                            continue;
                        }
                    }
                    "proxy_request_header_replace" => {
                        if let (Some(name), Some(value)) = (name, value) {
                            ferronconf::Statement::Directive(ferronconf::Directive {
                                name: "request_header".to_string(),
                                args: vec![
                                    ferronconf::Value::String(
                                        name,
                                        ferronconf::Span { line: 0, column: 0 },
                                    ),
                                    value,
                                ],
                                block: None,
                                span: ferronconf::Span { line: 0, column: 0 },
                            })
                        } else {
                            continue;
                        }
                    }
                    _ => unreachable!(),
                };
                nested_directives
                    .entry("proxy")
                    .or_insert_with(|| ferronconf::Block {
                        statements: vec![],
                        span: ferronconf::Span { line: 0, column: 0 },
                    })
                    .statements
                    .push(directive);
            }
            "proxy_concurrent_conns" | "auth_to_concurrent_conns" => {
                let val = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::Integer(i) => Some(*i as i64),
                    _ => None,
                });
                if let Some(val) = val {
                    statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                        name: node.name().to_string(),
                        args: vec![ferronconf::Value::Integer(
                            val,
                            ferronconf::Span { line: 0, column: 0 },
                        )],
                        block: None,
                        span: ferronconf::Span { line: 0, column: 0 },
                    }));
                }
            }
            "forward_proxy_auth" => {
                let mut auth_block = ferronconf::Block {
                    statements: vec![],
                    span: ferronconf::Span { line: 0, column: 0 },
                };
                for entry in &node.entries[1..] {
                    if let Some(key) = entry.key() {
                        if let kdlite::dom::Value::String(s) = &entry.value {
                            auth_block.statements.push(ferronconf::Statement::Directive(
                                ferronconf::Directive {
                                    name: key.to_string(),
                                    args: vec![ferronconf::Value::String(
                                        s.to_string(),
                                        ferronconf::Span { line: 0, column: 0 },
                                    )],
                                    block: None,
                                    span: ferronconf::Span { line: 0, column: 0 },
                                },
                            ));
                        }
                    }
                }
                statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                    name: "basic_auth".to_string(),
                    args: vec![],
                    block: Some(auth_block),
                    span: ferronconf::Span { line: 0, column: 0 },
                }));
            }

            "root" => {
                let path = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                if let Some(path) = path {
                    statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                        name: "root".to_string(),
                        args: vec![ferronconf::Value::String(
                            path,
                            ferronconf::Span { line: 0, column: 0 },
                        )],
                        block: None,
                        span: ferronconf::Span { line: 0, column: 0 },
                    }));
                }
            }
            "etag" | "compressed" | "precompressed" | "directory_listing"
            | "dynamic_compressed" => {
                let enabled = node.entries.first().is_none_or(|e| match e.value {
                    kdlite::dom::Value::Bool(b) => b,
                    _ => true,
                });
                statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                    name: node.name().to_string(),
                    args: vec![ferronconf::Value::Boolean(
                        enabled,
                        ferronconf::Span { line: 0, column: 0 },
                    )],
                    block: None,
                    span: ferronconf::Span { line: 0, column: 0 },
                }));
            }
            "mime_type" => {
                let ext = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                let mime = node.entries.get(1).and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                if let (Some(ext), Some(mime)) = (ext, mime) {
                    statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                        name: "mime_type".to_string(),
                        args: vec![
                            ferronconf::Value::String(ext, ferronconf::Span { line: 0, column: 0 }),
                            ferronconf::Value::String(
                                mime,
                                ferronconf::Span { line: 0, column: 0 },
                            ),
                        ],
                        block: None,
                        span: ferronconf::Span { line: 0, column: 0 },
                    }));
                }
            }
            "index" => {
                let files = node
                    .entries
                    .iter()
                    .filter_map(|e| match &e.value {
                        kdlite::dom::Value::String(s) => Some(ferronconf::Value::String(
                            s.to_string(),
                            ferronconf::Span { line: 0, column: 0 },
                        )),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                    name: "index".to_string(),
                    args: files,
                    block: None,
                    span: ferronconf::Span { line: 0, column: 0 },
                }));
            }
            "cache" => {
                let enabled = node.entries.first().map_or(true, |e| match e.value {
                    kdlite::dom::Value::Bool(b) => b,
                    _ => true,
                });
                statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                    name: "cache".to_string(),
                    args: vec![ferronconf::Value::Boolean(
                        enabled,
                        ferronconf::Span { line: 0, column: 0 },
                    )],
                    block: None,
                    span: ferronconf::Span { line: 0, column: 0 },
                }));
            }
            "cache_max_entries" => {
                let val = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::Integer(i) => Some(*i as i64),
                    _ => None,
                });
                if let Some(val) = val {
                    nested_directives
                        .entry("cache")
                        .or_insert_with(|| ferronconf::Block {
                            statements: vec![],
                            span: ferronconf::Span { line: 0, column: 0 },
                        })
                        .statements
                        .push(ferronconf::Statement::Directive(ferronconf::Directive {
                            name: "max_entries".to_string(),
                            args: vec![ferronconf::Value::Integer(
                                val,
                                ferronconf::Span { line: 0, column: 0 },
                            )],
                            block: None,
                            span: ferronconf::Span { line: 0, column: 0 },
                        }));
                }
            }
            "cache_max_response_size" => {
                let val = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::Integer(i) => Some(*i as i64),
                    _ => None,
                });
                if let Some(val) = val {
                    nested_directives
                        .entry("cache")
                        .or_insert_with(|| ferronconf::Block {
                            statements: vec![],
                            span: ferronconf::Span { line: 0, column: 0 },
                        })
                        .statements
                        .push(ferronconf::Statement::Directive(ferronconf::Directive {
                            name: "max_response_size".to_string(),
                            args: vec![ferronconf::Value::Integer(
                                val,
                                ferronconf::Span { line: 0, column: 0 },
                            )],
                            block: None,
                            span: ferronconf::Span { line: 0, column: 0 },
                        }));
                }
            }
            "cache_vary" => {
                let headers = node
                    .entries
                    .iter()
                    .filter_map(|e| match &e.value {
                        kdlite::dom::Value::String(s) => Some(ferronconf::Value::String(
                            s.to_string(),
                            ferronconf::Span { line: 0, column: 0 },
                        )),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                nested_directives
                    .entry("cache")
                    .or_insert_with(|| ferronconf::Block {
                        statements: vec![],
                        span: ferronconf::Span { line: 0, column: 0 },
                    })
                    .statements
                    .push(ferronconf::Statement::Directive(ferronconf::Directive {
                        name: "vary".to_string(),
                        args: headers,
                        block: None,
                        span: ferronconf::Span { line: 0, column: 0 },
                    }));
            }
            "cache_ignore" => {
                let headers = node
                    .entries
                    .iter()
                    .filter_map(|e| match &e.value {
                        kdlite::dom::Value::String(s) => Some(ferronconf::Value::String(
                            s.to_string(),
                            ferronconf::Span { line: 0, column: 0 },
                        )),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                nested_directives
                    .entry("cache")
                    .or_insert_with(|| ferronconf::Block {
                        statements: vec![],
                        span: ferronconf::Span { line: 0, column: 0 },
                    })
                    .statements
                    .push(ferronconf::Statement::Directive(ferronconf::Directive {
                        name: "ignore".to_string(),
                        args: headers,
                        block: None,
                        span: ferronconf::Span { line: 0, column: 0 },
                    }));
            }
            "file_cache_control" => {
                let val = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                if let Some(val) = val {
                    statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                        name: "file_cache_control".to_string(),
                        args: vec![ferronconf::Value::String(
                            val,
                            ferronconf::Span { line: 0, column: 0 },
                        )],
                        block: None,
                        span: ferronconf::Span { line: 0, column: 0 },
                    }));
                }
            }
            "replace" => {
                let search = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                let replace = node.entries.get(1).and_then(|e| match &e.value {
                    kdlite::dom::Value::String(s) => Some(s.to_string()),
                    _ => None,
                });
                let once = node
                    .entries
                    .iter()
                    .find(|e| e.key() == Some("once"))
                    .and_then(|e| match &e.value {
                        kdlite::dom::Value::Bool(b) => Some(*b),
                        _ => None,
                    })
                    .unwrap_or(true);

                if let (Some(search), Some(replace)) = (search, replace) {
                    let mut replace_block = ferronconf::Block {
                        statements: vec![],
                        span: ferronconf::Span { line: 0, column: 0 },
                    };
                    if !once {
                        replace_block
                            .statements
                            .push(ferronconf::Statement::Directive(ferronconf::Directive {
                                name: "once".to_string(),
                                args: vec![ferronconf::Value::Boolean(
                                    false,
                                    ferronconf::Span { line: 0, column: 0 },
                                )],
                                block: None,
                                span: ferronconf::Span { line: 0, column: 0 },
                            }));
                    }
                    statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                        name: "replace".to_string(),
                        args: vec![
                            ferronconf::Value::String(
                                search,
                                ferronconf::Span { line: 0, column: 0 },
                            ),
                            ferronconf::Value::String(
                                replace,
                                ferronconf::Span { line: 0, column: 0 },
                            ),
                        ],
                        block: if !once { Some(replace_block) } else { None },
                        span: ferronconf::Span { line: 0, column: 0 },
                    }));
                }
            }
            "replace_last_modified" => {
                let val = node.entries.first().map_or(true, |e| match e.value {
                    kdlite::dom::Value::Bool(b) => b,
                    _ => true,
                });
                statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                    name: "replace_last_modified".to_string(),
                    args: vec![ferronconf::Value::Boolean(
                        val,
                        ferronconf::Span { line: 0, column: 0 },
                    )],
                    block: None,
                    span: ferronconf::Span { line: 0, column: 0 },
                }));
            }
            "replace_filter_types" => {
                let types = node
                    .entries
                    .iter()
                    .filter_map(|e| match &e.value {
                        kdlite::dom::Value::String(s) => Some(ferronconf::Value::String(
                            s.to_string(),
                            ferronconf::Span { line: 0, column: 0 },
                        )),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                    name: "replace_filter_types".to_string(),
                    args: types,
                    block: None,
                    span: ferronconf::Span { line: 0, column: 0 },
                }));
            }

            // Conditionals
            "if" => {
                let condition_name = node
                    .entries
                    .first()
                    .and_then(|e| match &e.value {
                        kdlite::dom::Value::String(v) => Some(v),
                        _ => None,
                    })
                    .ok_or_else(|| anyhow::anyhow!("if condition must be a string name"))?;
                statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                    name: "if".to_string(),
                    args: vec![ferronconf::Value::String(
                        condition_name.to_string(),
                        ferronconf::Span { line: 0, column: 0 },
                    )],
                    block: Some(process_block(
                        node.children
                            .as_ref()
                            .ok_or_else(|| anyhow::anyhow!("if block must have children"))?,
                        snippets,
                    )?),
                    span: ferronconf::Span { line: 0, column: 0 },
                }));
            }
            "if_not" => {
                let condition_name = node
                    .entries
                    .first()
                    .and_then(|e| match &e.value {
                        kdlite::dom::Value::String(v) => Some(v),
                        _ => None,
                    })
                    .ok_or_else(|| anyhow::anyhow!("if_not condition must be a string name"))?;
                statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                    name: "if_not".to_string(),
                    args: vec![ferronconf::Value::String(
                        condition_name.to_string(),
                        ferronconf::Span { line: 0, column: 0 },
                    )],
                    block: Some(process_block(
                        node.children
                            .as_ref()
                            .ok_or_else(|| anyhow::anyhow!("if_not block must have children"))?,
                        snippets,
                    )?),
                    span: ferronconf::Span { line: 0, column: 0 },
                }));
            }
            "error_config" => {
                let condition_name = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::Integer(v) => Some(*v),
                    _ => None,
                });
                statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                    name: "handle_error".to_string(),
                    args: if let Some(condition_name) = condition_name {
                        vec![ferronconf::Value::Integer(
                            condition_name as i64,
                            ferronconf::Span { line: 0, column: 0 },
                        )]
                    } else {
                        vec![]
                    },
                    block: Some(process_block(
                        node.children.as_ref().ok_or_else(|| {
                            anyhow::anyhow!("error_config block must have children")
                        })?,
                        snippets,
                    )?),
                    span: ferronconf::Span { line: 0, column: 0 },
                }));
            }
            "condition" => {
                let condition_name = node
                    .entries
                    .first()
                    .and_then(|e| match &e.value {
                        kdlite::dom::Value::String(v) => Some(v),
                        _ => None,
                    })
                    .ok_or_else(|| anyhow::anyhow!("condition must be a string name"))?;
                let mut ferron3_conditions: Vec<ferronconf::MatcherExpression> = vec![];
                if let Some(children) = &node.children {
                    let mut nodes = children.nodes.iter().cloned().collect::<VecDeque<_>>();
                    while let Some(child) = nodes.pop_front() {
                        match child.name() {
                            // Snippets
                            "use" => {
                                let snippet_name = child
                                    .entries
                                    .first()
                                    .and_then(|e| match &e.value {
                                        kdlite::dom::Value::String(v) => Some(v),
                                        _ => None,
                                    })
                                    .ok_or_else(|| {
                                        anyhow::anyhow!("use directive must have a snippet name")
                                    })?;
                                if let Some(snippet) =
                                    snippets.get(&snippet_name.to_string()).cloned()
                                {
                                    let mut new_nodes =
                                        snippet.nodes.iter().cloned().collect::<VecDeque<_>>();
                                    new_nodes.extend(nodes);
                                    nodes = new_nodes;
                                } else {
                                    return Err(anyhow::anyhow!(
                                        "snippet not found: {}",
                                        snippet_name
                                    ));
                                }
                            }

                            "is_remote_ip" => {
                                let ips = child
                                    .entries
                                    .iter()
                                    .filter_map(|e| match &e.value {
                                        kdlite::dom::Value::String(v) => Some(v),
                                        _ => None,
                                    })
                                    .map(|v| regex_syntax::escape(v))
                                    .collect::<Vec<_>>()
                                    .join("|");
                                ferron3_conditions.push(ferronconf::MatcherExpression {
                                    left: ferronconf::Operand::Identifier(
                                        vec!["remote".to_string(), "ip".to_string()],
                                        ferronconf::Span { line: 0, column: 0 },
                                    ),
                                    right: ferronconf::Operand::String(
                                        format!("\\b(?:{ips})\\b"),
                                        ferronconf::Span { line: 0, column: 0 },
                                    ),
                                    op: ferronconf::Operator::Regex,
                                    span: ferronconf::Span { line: 0, column: 0 },
                                })
                            }
                            "is_forwarded_for" => {
                                let ips = child
                                    .entries
                                    .iter()
                                    .filter_map(|e| match &e.value {
                                        kdlite::dom::Value::String(v) => Some(v),
                                        _ => None,
                                    })
                                    .map(|v| regex_syntax::escape(v))
                                    .collect::<Vec<_>>()
                                    .join("|");
                                ferron3_conditions.push(ferronconf::MatcherExpression {
                                    left: ferronconf::Operand::Identifier(
                                        vec![
                                            "request".to_string(),
                                            "header".to_string(),
                                            "x_forwarded_for".to_string(),
                                        ],
                                        ferronconf::Span { line: 0, column: 0 },
                                    ),
                                    right: ferronconf::Operand::String(
                                        format!("\\b(?:{ips})\\b"),
                                        ferronconf::Span { line: 0, column: 0 },
                                    ),
                                    op: ferronconf::Operator::Regex,
                                    span: ferronconf::Span { line: 0, column: 0 },
                                })
                            }
                            "is_not_remote_ip" => {
                                let ips = child
                                    .entries
                                    .iter()
                                    .filter_map(|e| match &e.value {
                                        kdlite::dom::Value::String(v) => Some(v),
                                        _ => None,
                                    })
                                    .map(|v| regex_syntax::escape(v))
                                    .collect::<Vec<_>>()
                                    .join("|");
                                ferron3_conditions.push(ferronconf::MatcherExpression {
                                    left: ferronconf::Operand::Identifier(
                                        vec!["remote".to_string(), "ip".to_string()],
                                        ferronconf::Span { line: 0, column: 0 },
                                    ),
                                    right: ferronconf::Operand::String(
                                        format!("\\b(?:{ips})\\b"),
                                        ferronconf::Span { line: 0, column: 0 },
                                    ),
                                    op: ferronconf::Operator::NotRegex,
                                    span: ferronconf::Span { line: 0, column: 0 },
                                })
                            }
                            "is_not_forwarded_for" => {
                                let ips = child
                                    .entries
                                    .iter()
                                    .filter_map(|e| match &e.value {
                                        kdlite::dom::Value::String(v) => Some(v),
                                        _ => None,
                                    })
                                    .map(|v| regex_syntax::escape(v))
                                    .collect::<Vec<_>>()
                                    .join("|");
                                ferron3_conditions.push(ferronconf::MatcherExpression {
                                    left: ferronconf::Operand::Identifier(
                                        vec![
                                            "request".to_string(),
                                            "header".to_string(),
                                            "x_forwarded_for".to_string(),
                                        ],
                                        ferronconf::Span { line: 0, column: 0 },
                                    ),
                                    right: ferronconf::Operand::String(
                                        format!("\\b(?:{ips})\\b"),
                                        ferronconf::Span { line: 0, column: 0 },
                                    ),
                                    op: ferronconf::Operator::NotRegex,
                                    span: ferronconf::Span { line: 0, column: 0 },
                                })
                            }
                            "is_equal" => {
                                let left = child
                                    .entries
                                    .first()
                                    .and_then(|e| match &e.value {
                                        kdlite::dom::Value::String(v) => {
                                            Some(convert_placeholders_into_interpolated_strings(v))
                                        }
                                        _ => None,
                                    })
                                    .and_then(|v| match v {
                                        ferronconf::Value::String(v, s) => {
                                            Some(ferronconf::Operand::String(v, s))
                                        }
                                        ferronconf::Value::InterpolatedString(mut v, s) => {
                                            if v.len() == 1 {
                                                let popped =
                                                    v.pop().expect("should have exactly one part");
                                                match popped {
                                                    ferronconf::StringPart::Expression(e) => {
                                                        Some(ferronconf::Operand::Identifier(e, s))
                                                    }
                                                    ferronconf::StringPart::Literal(l) => {
                                                        Some(ferronconf::Operand::String(l, s))
                                                    }
                                                }
                                            } else {
                                                let r = v
                                                    .into_iter()
                                                    .map(|p| p.as_str())
                                                    .collect::<String>();
                                                Some(ferronconf::Operand::String(r, s))
                                            }
                                        }
                                        _ => None,
                                    })
                                    .ok_or_else(|| {
                                        anyhow::anyhow!("left operand is not a string")
                                    })?;
                                let right = child
                                    .entries
                                    .get(1)
                                    .and_then(|e| match &e.value {
                                        kdlite::dom::Value::String(v) => {
                                            Some(convert_placeholders_into_interpolated_strings(v))
                                        }
                                        _ => None,
                                    })
                                    .and_then(|v| match v {
                                        ferronconf::Value::String(v, s) => {
                                            Some(ferronconf::Operand::String(v, s))
                                        }
                                        ferronconf::Value::InterpolatedString(mut v, s) => {
                                            if v.len() == 1 {
                                                let popped =
                                                    v.pop().expect("should have exactly one part");
                                                match popped {
                                                    ferronconf::StringPart::Expression(e) => {
                                                        Some(ferronconf::Operand::Identifier(e, s))
                                                    }
                                                    ferronconf::StringPart::Literal(l) => {
                                                        Some(ferronconf::Operand::String(l, s))
                                                    }
                                                }
                                            } else {
                                                let r = v
                                                    .into_iter()
                                                    .map(|p| p.as_str())
                                                    .collect::<String>();
                                                Some(ferronconf::Operand::String(r, s))
                                            }
                                        }
                                        _ => None,
                                    })
                                    .ok_or_else(|| {
                                        anyhow::anyhow!("right operand is not a string")
                                    })?;
                                ferron3_conditions.push(ferronconf::MatcherExpression {
                                    left,
                                    right,
                                    op: ferronconf::Operator::Eq,
                                    span: ferronconf::Span { line: 0, column: 0 },
                                })
                            }
                            "is_not_equal" => {
                                let left = child
                                    .entries
                                    .get(1)
                                    .and_then(|e| match &e.value {
                                        kdlite::dom::Value::String(v) => {
                                            Some(convert_placeholders_into_interpolated_strings(v))
                                        }
                                        _ => None,
                                    })
                                    .and_then(|v| match v {
                                        ferronconf::Value::String(v, s) => {
                                            Some(ferronconf::Operand::String(v, s))
                                        }
                                        ferronconf::Value::InterpolatedString(mut v, s) => {
                                            if v.len() == 1 {
                                                let popped =
                                                    v.pop().expect("should have exactly one part");
                                                match popped {
                                                    ferronconf::StringPart::Expression(e) => {
                                                        Some(ferronconf::Operand::Identifier(e, s))
                                                    }
                                                    ferronconf::StringPart::Literal(l) => {
                                                        Some(ferronconf::Operand::String(l, s))
                                                    }
                                                }
                                            } else {
                                                let r = v
                                                    .into_iter()
                                                    .map(|p| p.as_str())
                                                    .collect::<String>();
                                                Some(ferronconf::Operand::String(r, s))
                                            }
                                        }
                                        _ => None,
                                    })
                                    .ok_or_else(|| {
                                        anyhow::anyhow!("left operand is not a string")
                                    })?;
                                let right = child
                                    .entries
                                    .get(1)
                                    .and_then(|e| match &e.value {
                                        kdlite::dom::Value::String(v) => {
                                            Some(convert_placeholders_into_interpolated_strings(v))
                                        }
                                        _ => None,
                                    })
                                    .and_then(|v| match v {
                                        ferronconf::Value::String(v, s) => {
                                            Some(ferronconf::Operand::String(v, s))
                                        }
                                        ferronconf::Value::InterpolatedString(mut v, s) => {
                                            if v.len() == 1 {
                                                let popped =
                                                    v.pop().expect("should have exactly one part");
                                                match popped {
                                                    ferronconf::StringPart::Expression(e) => {
                                                        Some(ferronconf::Operand::Identifier(e, s))
                                                    }
                                                    ferronconf::StringPart::Literal(l) => {
                                                        Some(ferronconf::Operand::String(l, s))
                                                    }
                                                }
                                            } else {
                                                let r = v
                                                    .into_iter()
                                                    .map(|p| p.as_str())
                                                    .collect::<String>();
                                                Some(ferronconf::Operand::String(r, s))
                                            }
                                        }
                                        _ => None,
                                    })
                                    .ok_or_else(|| {
                                        anyhow::anyhow!("right operand is not a string")
                                    })?;
                                ferron3_conditions.push(ferronconf::MatcherExpression {
                                    left,
                                    right,
                                    op: ferronconf::Operator::NotEq,
                                    span: ferronconf::Span { line: 0, column: 0 },
                                })
                            }
                            "is_regex" => {
                                let left = child
                                    .entries
                                    .first()
                                    .and_then(|e| match &e.value {
                                        kdlite::dom::Value::String(v) => {
                                            Some(convert_placeholders_into_interpolated_strings(v))
                                        }
                                        _ => None,
                                    })
                                    .and_then(|v| match v {
                                        ferronconf::Value::String(v, s) => {
                                            Some(ferronconf::Operand::String(v, s))
                                        }
                                        ferronconf::Value::InterpolatedString(mut v, s) => {
                                            if v.len() == 1 {
                                                let popped =
                                                    v.pop().expect("should have exactly one part");
                                                match popped {
                                                    ferronconf::StringPart::Expression(e) => {
                                                        Some(ferronconf::Operand::Identifier(e, s))
                                                    }
                                                    ferronconf::StringPart::Literal(l) => {
                                                        Some(ferronconf::Operand::String(l, s))
                                                    }
                                                }
                                            } else {
                                                let r = v
                                                    .into_iter()
                                                    .map(|p| p.as_str())
                                                    .collect::<String>();
                                                Some(ferronconf::Operand::String(r, s))
                                            }
                                        }
                                        _ => None,
                                    })
                                    .ok_or_else(|| {
                                        anyhow::anyhow!("left operand is not a string")
                                    })?;
                                let case_insensitive = child
                                    .entries
                                    .iter()
                                    .find(|e| e.key() == Some("case_insensitive"))
                                    .and_then(|e| match &e.value {
                                        kdlite::dom::Value::Bool(v) => Some(*v),
                                        _ => None,
                                    })
                                    .unwrap_or(false);
                                let right = child
                                    .entries
                                    .get(1)
                                    .and_then(|e| match &e.value {
                                        kdlite::dom::Value::String(v) => {
                                            Some(ferronconf::Operand::String(
                                                if case_insensitive {
                                                    format!("(?i){}", v)
                                                } else {
                                                    v.to_string()
                                                },
                                                ferronconf::Span { line: 0, column: 0 },
                                            ))
                                        }
                                        _ => None,
                                    })
                                    .ok_or_else(|| {
                                        anyhow::anyhow!("right operand is not a string")
                                    })?;
                                ferron3_conditions.push(ferronconf::MatcherExpression {
                                    left,
                                    right,
                                    op: ferronconf::Operator::Regex,
                                    span: ferronconf::Span { line: 0, column: 0 },
                                })
                            }
                            "is_not_regex" => {
                                let left = child
                                    .entries
                                    .first()
                                    .and_then(|e| match &e.value {
                                        kdlite::dom::Value::String(v) => {
                                            Some(convert_placeholders_into_interpolated_strings(v))
                                        }
                                        _ => None,
                                    })
                                    .and_then(|v| match v {
                                        ferronconf::Value::String(v, s) => {
                                            Some(ferronconf::Operand::String(v, s))
                                        }
                                        ferronconf::Value::InterpolatedString(mut v, s) => {
                                            if v.len() == 1 {
                                                let popped =
                                                    v.pop().expect("should have exactly one part");
                                                match popped {
                                                    ferronconf::StringPart::Expression(e) => {
                                                        Some(ferronconf::Operand::Identifier(e, s))
                                                    }
                                                    ferronconf::StringPart::Literal(l) => {
                                                        Some(ferronconf::Operand::String(l, s))
                                                    }
                                                }
                                            } else {
                                                let r = v
                                                    .into_iter()
                                                    .map(|p| p.as_str())
                                                    .collect::<String>();
                                                Some(ferronconf::Operand::String(r, s))
                                            }
                                        }
                                        _ => None,
                                    })
                                    .ok_or_else(|| {
                                        anyhow::anyhow!("left operand is not a string")
                                    })?;
                                let case_insensitive = child
                                    .entries
                                    .iter()
                                    .find(|e| e.key() == Some("case_insensitive"))
                                    .and_then(|e| match &e.value {
                                        kdlite::dom::Value::Bool(v) => Some(*v),
                                        _ => None,
                                    })
                                    .unwrap_or(false);
                                let right = child
                                    .entries
                                    .get(1)
                                    .and_then(|e| match &e.value {
                                        kdlite::dom::Value::String(v) => {
                                            Some(ferronconf::Operand::String(
                                                if case_insensitive {
                                                    format!("(?i){}", v)
                                                } else {
                                                    v.to_string()
                                                },
                                                ferronconf::Span { line: 0, column: 0 },
                                            ))
                                        }
                                        _ => None,
                                    })
                                    .ok_or_else(|| {
                                        anyhow::anyhow!("right operand is not a string")
                                    })?;
                                ferron3_conditions.push(ferronconf::MatcherExpression {
                                    left,
                                    right,
                                    op: ferronconf::Operator::NotRegex,
                                    span: ferronconf::Span { line: 0, column: 0 },
                                })
                            }
                            "is_rego" | "set_constant" => {
                                // No-ops in Ferron 3
                                // - is_rego is deprecated in Ferron 2
                                // - set_constant would be unused aside `is_language` in Ferron 2
                            }
                            "is_language" => {
                                let language = child
                                    .entries
                                    .first()
                                    .and_then(|e| match &e.value {
                                        kdlite::dom::Value::String(v) => {
                                            Some(ferronconf::Operand::String(
                                                v.to_string(),
                                                ferronconf::Span { line: 0, column: 0 },
                                            ))
                                        }
                                        _ => None,
                                    })
                                    .ok_or_else(|| {
                                        anyhow::anyhow!("right operand is not a string")
                                    })?;
                                ferron3_conditions.push(ferronconf::MatcherExpression {
                                    left: language,
                                    right: ferronconf::Operand::Identifier(
                                        vec![
                                            "request".to_string(),
                                            "header".to_string(),
                                            "accept_language".to_string(),
                                        ],
                                        ferronconf::Span { line: 0, column: 0 },
                                    ),
                                    op: ferronconf::Operator::In,
                                    span: ferronconf::Span { line: 0, column: 0 },
                                })
                            }
                            _ => {} // Unsupported subcondition
                        }
                    }
                }
                statements.push(ferronconf::Statement::MatchBlock(ferronconf::MatchBlock {
                    matcher: condition_name.to_string(),
                    expr: ferron3_conditions,
                    span: ferronconf::Span { line: 0, column: 0 },
                }));
            }
            "location" => {
                let location_path = node
                    .entries
                    .first()
                    .and_then(|e| match &e.value {
                        kdlite::dom::Value::String(v) => Some(v),
                        _ => None,
                    })
                    .ok_or_else(|| anyhow::anyhow!("location condition must be a string name"))?;
                let remove_base = node
                    .entries
                    .iter()
                    .find(|e| e.key() == Some("remove_base"))
                    .and_then(|e| match &e.value {
                        kdlite::dom::Value::Bool(v) => Some(*v),
                        _ => None,
                    })
                    .unwrap_or(false);
                if remove_base {
                    statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                        name: "location".to_string(),
                        args: vec![ferronconf::Value::String(
                            location_path.to_string(),
                            ferronconf::Span { line: 0, column: 0 },
                        )],
                        block: Some(process_block(
                            node.children.as_ref().ok_or_else(|| {
                                anyhow::anyhow!("location block must have children")
                            })?,
                            snippets,
                        )?),
                        span: ferronconf::Span { line: 0, column: 0 },
                    }));
                } else {
                    // Since in Ferron 3 "location" blocks always strip base paths,
                    // we have to use "if" with custom "match" block.
                    let match_id = rand::random::<u64>();
                    let match_id_str = format!("ferron2__location_{:x}", match_id);
                    statements.push(ferronconf::Statement::MatchBlock(ferronconf::MatchBlock {
                        matcher: match_id_str.clone(),
                        expr: vec![ferronconf::MatcherExpression {
                            left: ferronconf::Operand::Identifier(
                                vec!["request".to_string(), "uri".to_string(), "path".to_string()],
                                ferronconf::Span { line: 0, column: 0 },
                            ),
                            right: ferronconf::Operand::String(
                                format!(
                                    "^{}(?:$|/)",
                                    regex_syntax::escape(location_path.trim_end_matches('/'))
                                ),
                                ferronconf::Span { line: 0, column: 0 },
                            ),
                            op: ferronconf::Operator::Regex,
                            span: ferronconf::Span { line: 0, column: 0 },
                        }],
                        span: ferronconf::Span { line: 0, column: 0 },
                    }));
                    statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                        name: "if".to_string(),
                        args: vec![ferronconf::Value::String(
                            match_id_str.to_string(),
                            ferronconf::Span { line: 0, column: 0 },
                        )],
                        block: Some(process_block(
                            node.children.as_ref().ok_or_else(|| {
                                anyhow::anyhow!("location block must have children")
                            })?,
                            snippets,
                        )?),
                        span: ferronconf::Span { line: 0, column: 0 },
                    }));
                }
            }

            // Snippets
            "use" => {
                let value = node
                    .entries
                    .first()
                    .and_then(|e| match &e.value {
                        kdlite::dom::Value::String(value) => Some(value),
                        _ => None,
                    })
                    .ok_or(anyhow::anyhow!("snippet node must have a value"))?;
                if let Some(snippet) = snippets.get(&value.to_string()).cloned() {
                    statements.extend(process_block(&snippet, snippets)?.statements);
                } else {
                    return Err(anyhow::anyhow!("snippet not found: {}", value));
                }
            }

            // HTTP protocol & performance
            "default_http_port" => {
                let default_http_port = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::Integer(v) => Some(*v as i64),
                    _ => None,
                });
                statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                    name: "default_http_port".to_string(),
                    args: vec![if let Some(default_http_port) = default_http_port {
                        ferronconf::Value::Integer(
                            default_http_port,
                            ferronconf::Span { line: 0, column: 0 },
                        )
                    } else {
                        ferronconf::Value::Boolean(false, ferronconf::Span { line: 0, column: 0 })
                    }],
                    block: None,
                    span: ferronconf::Span { line: 0, column: 0 },
                }));
            }
            "default_https_port" => {
                let default_http_port = node.entries.first().and_then(|e| match &e.value {
                    kdlite::dom::Value::Integer(v) => Some(*v as i64),
                    _ => None,
                });
                statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
                    name: "default_http_port".to_string(),
                    args: vec![if let Some(default_http_port) = default_http_port {
                        ferronconf::Value::Integer(
                            default_http_port,
                            ferronconf::Span { line: 0, column: 0 },
                        )
                    } else {
                        ferronconf::Value::Boolean(false, ferronconf::Span { line: 0, column: 0 })
                    }],
                    block: None,
                    span: ferronconf::Span { line: 0, column: 0 },
                }));
            }
            "protocols" => {
                let protocols = node
                    .entries
                    .iter()
                    .filter_map(|e| match &e.value {
                        kdlite::dom::Value::String(s) => Some(s.to_string()),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                nested_directives
                    .entry("http")
                    .or_insert_with(|| ferronconf::Block {
                        statements: vec![],
                        span: ferronconf::Span { line: 0, column: 0 },
                    })
                    .statements
                    .push(ferronconf::Statement::Directive(ferronconf::Directive {
                        name: "protocols".to_string(),
                        args: protocols
                            .iter()
                            .map(|p| {
                                ferronconf::Value::String(
                                    p.to_string(),
                                    ferronconf::Span { line: 0, column: 0 },
                                )
                            })
                            .collect::<Vec<_>>(),
                        block: None,
                        span: ferronconf::Span { line: 0, column: 0 },
                    }));
            }
            "timeout" => {
                let timeout = node
                    .entries
                    .first()
                    .and_then(|e| duration_kdl_to_ferron(&e.value))
                    .ok_or_else(|| {
                        anyhow::anyhow!("timeout directive must have timeout duration")
                    })?;
                nested_directives
                    .entry("http")
                    .or_insert_with(|| ferronconf::Block {
                        statements: vec![],
                        span: ferronconf::Span { line: 0, column: 0 },
                    })
                    .statements
                    .push(ferronconf::Statement::Directive(ferronconf::Directive {
                        name: "timeout".to_string(),
                        args: vec![timeout],
                        block: None,
                        span: ferronconf::Span { line: 0, column: 0 },
                    }));
            }
            "h2_initial_window_size"
            | "h2_max_frame_size"
            | "h2_max_concurrent_streams"
            | "h2_max_header_list_size" => {
                let value = node
                    .entries
                    .first()
                    .and_then(|e| duration_kdl_to_ferron(&e.value))
                    .ok_or_else(|| {
                        anyhow::anyhow!("HTTP/2 integer directive must have integer value")
                    })?;
                nested_directives
                    .entry("http")
                    .or_insert_with(|| ferronconf::Block {
                        statements: vec![],
                        span: ferronconf::Span { line: 0, column: 0 },
                    })
                    .statements
                    .push(ferronconf::Statement::Directive(ferronconf::Directive {
                        name: node.name().to_string(),
                        args: vec![value],
                        block: None,
                        span: ferronconf::Span { line: 0, column: 0 },
                    }));
            }
            "h2_enable_connect_protocol" | "protocol_proxy" => {
                let value = node.entries.first().is_some_and(|e| match e.value {
                    kdlite::dom::Value::Bool(b) => b,
                    _ => true,
                });
                nested_directives
                    .entry("http")
                    .or_insert_with(|| ferronconf::Block {
                        statements: vec![],
                        span: ferronconf::Span { line: 0, column: 0 },
                    })
                    .statements
                    .push(ferronconf::Statement::Directive(ferronconf::Directive {
                        name: node.name().to_string(),
                        args: vec![ferronconf::Value::Boolean(
                            value,
                            ferronconf::Span { line: 0, column: 0 },
                        )],
                        block: None,
                        span: ferronconf::Span { line: 0, column: 0 },
                    }));
            }
            "buffer_request" | "buffer_response" => {
                let value = node
                    .entries
                    .first()
                    .and_then(|e| match e.value {
                        kdlite::dom::Value::Integer(i) => Some(i),
                        _ => None,
                    })
                    .ok_or_else(|| anyhow::anyhow!("Invalid value for buffer size"))?;
                nested_directives
                    .entry("http")
                    .or_insert_with(|| ferronconf::Block {
                        statements: vec![],
                        span: ferronconf::Span { line: 0, column: 0 },
                    })
                    .statements
                    .push(ferronconf::Statement::Directive(ferronconf::Directive {
                        name: node.name().to_string(),
                        args: vec![ferronconf::Value::Integer(
                            value as i64,
                            ferronconf::Span { line: 0, column: 0 },
                        )],
                        block: None,
                        span: ferronconf::Span { line: 0, column: 0 },
                    }));
            }

            // Networking & system
            "listen_ip" => {
                let timeout = node
                    .entries
                    .first()
                    .and_then(|e| match &e.value {
                        kdlite::dom::Value::String(s) => Some(s.to_string()),
                        _ => None,
                    })
                    .ok_or_else(|| {
                        anyhow::anyhow!("listen_ip directive must have a string value")
                    })?;
                nested_directives
                    .entry("tcp")
                    .or_insert_with(|| ferronconf::Block {
                        statements: vec![],
                        span: ferronconf::Span { line: 0, column: 0 },
                    })
                    .statements
                    .push(ferronconf::Statement::Directive(ferronconf::Directive {
                        name: "listen_ip".to_string(),
                        args: vec![ferronconf::Value::String(
                            timeout,
                            ferronconf::Span { line: 0, column: 0 },
                        )],
                        block: None,
                        span: ferronconf::Span { line: 0, column: 0 },
                    }));
            }
            "io_uring" => {
                let value = node.entries.first().is_some_and(|e| match e.value {
                    kdlite::dom::Value::Bool(i) => i,
                    _ => true,
                });
                nested_directives
                    .entry("runtime")
                    .or_insert_with(|| ferronconf::Block {
                        statements: vec![],
                        span: ferronconf::Span { line: 0, column: 0 },
                    })
                    .statements
                    .push(ferronconf::Statement::Directive(ferronconf::Directive {
                        name: "io_uring".to_string(),
                        args: vec![ferronconf::Value::Boolean(
                            value,
                            ferronconf::Span { line: 0, column: 0 },
                        )],
                        block: None,
                        span: ferronconf::Span { line: 0, column: 0 },
                    }));
            }
            "tcp_send_buffer" => {
                let default_http_port = node
                    .entries
                    .first()
                    .and_then(|e| match &e.value {
                        kdlite::dom::Value::Integer(v) => Some(*v as i64),
                        _ => None,
                    })
                    .ok_or_else(|| {
                        anyhow::anyhow!("tcp_send_buffer directive must have a buffer size")
                    })?;
                nested_directives
                    .entry("tcp")
                    .or_insert_with(|| ferronconf::Block {
                        statements: vec![],
                        span: ferronconf::Span { line: 0, column: 0 },
                    })
                    .statements
                    .push(ferronconf::Statement::Directive(ferronconf::Directive {
                        name: "send_buf".to_string(),
                        args: vec![ferronconf::Value::Integer(
                            default_http_port,
                            ferronconf::Span { line: 0, column: 0 },
                        )],
                        block: None,
                        span: ferronconf::Span { line: 0, column: 0 },
                    }));
            }
            "tcp_recv_buffer" => {
                let default_http_port = node
                    .entries
                    .first()
                    .and_then(|e| match &e.value {
                        kdlite::dom::Value::Integer(v) => Some(*v as i64),
                        _ => None,
                    })
                    .ok_or_else(|| {
                        anyhow::anyhow!("tcp_recv_buffer directive must have a buffer size")
                    })?;
                nested_directives
                    .entry("tcp")
                    .or_insert_with(|| ferronconf::Block {
                        statements: vec![],
                        span: ferronconf::Span { line: 0, column: 0 },
                    })
                    .statements
                    .push(ferronconf::Statement::Directive(ferronconf::Directive {
                        name: "recv_buf".to_string(),
                        args: vec![ferronconf::Value::Integer(
                            default_http_port,
                            ferronconf::Span { line: 0, column: 0 },
                        )],
                        block: None,
                        span: ferronconf::Span { line: 0, column: 0 },
                    }));
            }

            // Unsupported
            _ => {}
        }
    }

    for (key, block) in nested_directives {
        statements.push(ferronconf::Statement::Directive(ferronconf::Directive {
            name: key.to_string(),
            args: vec![],
            block: Some(block),
            span: ferronconf::Span { line: 0, column: 0 },
        }));
    }

    Ok(ferronconf::Block {
        statements,
        span: ferronconf::Span { line: 0, column: 0 },
    })
}

fn duration_kdl_to_ferron(duration: &kdlite::dom::Value) -> Option<ferronconf::Value> {
    match duration {
        kdlite::dom::Value::Integer(i) => Some(ferronconf::Value::String(
            {
                // Ferron 3 support "h" (hours), "m" (minutes), "s" (seconds) and "d" (days) suffixes
                // Ferron 2 durations are in milliseconds
                format!("{}s", i / 1000)
            },
            ferronconf::Span { line: 0, column: 0 },
        )),
        kdlite::dom::Value::String(s) => Some(ferronconf::Value::String(
            s.to_string(),
            ferronconf::Span { line: 0, column: 0 },
        )),
        _ => None,
    }
}

fn convert_placeholders_into_interpolated_strings(templated: &str) -> ferronconf::Value {
    let mut parts: Vec<ferronconf::StringPart> = Vec::new();
    let mut remaining = templated.to_string();
    while let Some(start) = remaining.find('{') {
        if let Some(end) = remaining[start + 1..].find('}') {
            let placeholder = &remaining[start + 1..start + end + 1];
            if start > 0 {
                parts.push(ferronconf::StringPart::Literal(
                    remaining[..start].to_string(),
                ));
            }
            let resolved: Option<String> = match placeholder {
                "path" => Some("request.uri.path".to_string()),
                "path_and_query" => Some("request.uri".to_string()),
                "method" => Some("request.method".to_string()),
                "version" => Some("request.version".to_string()),
                "scheme" => Some("request.scheme".to_string()),
                "client_ip" => Some("remote.ip".to_string()),
                "client_port" => Some("remote.port".to_string()),
                "client_ip_canonical" => Some("remote.ip".to_string()), // Automatically canonicalized
                "server_ip" => Some("server.ip".to_string()),
                "server_port" => Some("server.port".to_string()),
                "server_ip_canonical" => Some("server.ip".to_string()), // Automatically canonicalized
                s if s.starts_with("header:") => Some(format!(
                    "request.header.{}",
                    &s[7..].to_lowercase().replace("-", "_")
                )),
                _ => None,
            };
            if let Some(resolved) = resolved {
                parts.push(ferronconf::StringPart::Expression(
                    resolved.split('.').map(String::from).collect::<Vec<_>>(),
                ));
            } else {
                parts.push(ferronconf::StringPart::Literal(format!(
                    "{{{placeholder}}}",
                )));
            }
            remaining = remaining.split_off(start + end + 2);
        } else {
            break;
        }
    }
    if !remaining.is_empty() {
        parts.push(ferronconf::StringPart::Literal(remaining));
    }
    if parts.is_empty() {
        return ferronconf::Value::String("".to_string(), ferronconf::Span { line: 0, column: 0 });
    } else if parts.len() == 1 {
        if let ferronconf::StringPart::Literal(literal) = &parts[0] {
            return ferronconf::Value::String(
                literal.to_string(),
                ferronconf::Span { line: 0, column: 0 },
            );
        }
    }

    ferronconf::Value::InterpolatedString(parts, ferronconf::Span { line: 0, column: 0 })
}
