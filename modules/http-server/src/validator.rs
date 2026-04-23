use std::collections::HashMap;

use cidr::IpCidr;
use ferron_core::{config::ServerConfigurationValue, validate_directive, validate_nested};

pub struct HttpConfigurationValidator;

impl ferron_core::config::validator::ConfigurationValidator for HttpConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        used_directives: &mut std::collections::HashSet<String>,
        is_global: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Global-only directives (default port configuration)
        if is_global {
            validate_directive!(config, used_directives, default_http_port, optional args(1) => [
                ServerConfigurationValue::Number(_, _)
                    | ServerConfigurationValue::Boolean(_, _)
            ], {});

            validate_directive!(config, used_directives, default_https_port, optional args(1) => [
                ServerConfigurationValue::Number(_, _)
                    | ServerConfigurationValue::Boolean(_, _)
            ], {});
        }

        // TLS settings
        validate_directive!(config, used_directives, tls, optional
            args(1) => [ServerConfigurationValue::Boolean(_, _)]
            | args(2) => [
                ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _),
                ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)
            ],
            {
            validate_nested!(tls, provider, optional args(1) => [ServerConfigurationValue::String(_, _)]);

            // Session ticket keys configuration (validated at runtime by provider)
            validate_nested!(tls, ticket_keys, optional args(?) => [
                ServerConfigurationValue::String(_, _)
                    | ServerConfigurationValue::InterpolatedString(_, _)
                    | ServerConfigurationValue::Number(_, _)
                    | ServerConfigurationValue::Boolean(_, _)
            ]);
        });

        // HTTP settings
        validate_directive!(config, used_directives, http, no_args, {
            validate_nested!(http, protocols, args(*) => [ServerConfigurationValue::String(_, _)]);

            // OPTIONS * allowed methods
            validate_nested!(http, options_allowed_methods, args(1) => [
                ServerConfigurationValue::String(_, _)
                    | ServerConfigurationValue::InterpolatedString(_, _)
            ]);

            // Timeout
            validate_nested!(http, timeout, args(1) => [
                ServerConfigurationValue::Number(_, _)
                    | ServerConfigurationValue::Boolean(false, _)
                    | ServerConfigurationValue::String(_, _)
                    | ServerConfigurationValue::InterpolatedString(_, _)
            ]);

            // URL sanitization
            if is_global {
                validate_nested!(http, url_sanitize, optional args(1) => [ServerConfigurationValue::Boolean(_, _)]);
                validate_nested!(http, url_reject_backslash, optional args(1) => [ServerConfigurationValue::Boolean(_, _)]);
            }

            // HTTP/1.x settings
            validate_nested!(http, h1_enable_early_hints, optional args(1) => [ServerConfigurationValue::Boolean(_, _)]);

            // 103 Early Hints
            validate_nested!(http, early_hints, optional);

            // HTTP/2 settings
            validate_nested!(http, h2_initial_window_size, args(1) => [ServerConfigurationValue::Number(_, _)]);
            validate_nested!(http, h2_max_frame_size, args(1) => [ServerConfigurationValue::Number(_, _)]);
            validate_nested!(http, h2_max_concurrent_streams, args(1) => [ServerConfigurationValue::Number(_, _)]);
            validate_nested!(http, h2_max_header_list_size, args(1) => [ServerConfigurationValue::Number(_, _)]);
            validate_nested!(http, h2_enable_connect_protocol, optional args(1) => [ServerConfigurationValue::Boolean(_, _)]);

            // W3C Trace Context
            validate_nested!(http, trace, {
                validate_nested!(trace, generate, optional args(1) => [ServerConfigurationValue::Boolean(_, _)]);
                validate_nested!(trace, sampled, optional args(1) => [ServerConfigurationValue::Boolean(_, _)]);
            });
        });

        // Webroot
        validate_directive!(config, used_directives, root, args(1) => [
            ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)
        ], {});

        // Server administrator's email address
        validate_directive!(config, used_directives, admin_email, args(1) => [
            ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)
        ], {});

        // PROXY protocol
        validate_directive!(config, used_directives, protocol_proxy, optional args(1) => [
            ServerConfigurationValue::Boolean(_, _)
        ], {});

        // Observability aliases
        validate_directive!(config, used_directives, log, optional
            args(1) => [ServerConfigurationValue::Boolean(_, _)]
            | args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)],
            {
            validate_nested!(log, format, args(1) => ServerConfigurationValue::String(_, _));
            validate_nested!(log, access_log_rotate_size, optional args(1) => [ServerConfigurationValue::Number(_, _)]);
            validate_nested!(log, access_log_rotate_keep, optional args(1) => [ServerConfigurationValue::Number(_, _)]);
        });

        validate_directive!(config, used_directives, error_log, optional
            args(1) => [ServerConfigurationValue::Boolean(_, _)]
            | args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)],
            {
            validate_nested!(error_log, error_log_rotate_size, optional args(1) => [ServerConfigurationValue::Number(_, _)]);
            validate_nested!(error_log, error_log_rotate_keep, optional args(1) => [ServerConfigurationValue::Number(_, _)]);
        });

        validate_directive!(config, used_directives, console_log, optional
            args(1) => [ServerConfigurationValue::Boolean(_, _)],
            {
            validate_nested!(console_log, format, args(1) => ServerConfigurationValue::String(_, _));
        });

        // Index file names
        validate_directive!(config, used_directives, index, optional args(?), {});

        // Trailing slash redirect for directories
        validate_directive!(config, used_directives, trailing_slash_redirect, optional args(1) => [
            ServerConfigurationValue::Boolean(_, _)
        ], {});

        // HTTPS redirect toggle
        validate_directive!(config, used_directives, https_redirect, optional args(1) => [
            ServerConfigurationValue::Boolean(_, _)
        ], {});

        // Client IP from forwarded header
        if let Some(entries) = config.directives.get("client_ip_from_header") {
            used_directives.insert("client_ip_from_header".to_string());
            for entry in entries {
                if entry.args.len() != 1 {
                    return Err(format!(
                        "Invalid directive 'client_ip_from_header': expected 1 argument, got {}",
                        entry.args.len()
                    )
                    .into());
                }
                if !matches!(
                    entry.args.first(),
                    Some(ServerConfigurationValue::String(_, _))
                        | Some(ServerConfigurationValue::InterpolatedString(_, _))
                ) {
                    return Err(
                        "Invalid directive 'client_ip_from_header': argument type mismatch".into(),
                    );
                }

                if let Some(children) = &entry.children {
                    for directive_name in children.directives.keys() {
                        if directive_name != "trusted_proxy" {
                            return Err(format!(
                                "Invalid directive 'client_ip_from_header': unknown nested directive '{directive_name}'"
                            )
                            .into());
                        }
                    }

                    if let Some(trusted_proxy_entries) = children.directives.get("trusted_proxy") {
                        used_directives.insert("trusted_proxy".to_string());
                        for trusted_proxy_entry in trusted_proxy_entries {
                            if trusted_proxy_entry.args.is_empty() {
                                return Err(
                                    "Invalid directive 'trusted_proxy': expected at least one IP or CIDR"
                                        .into(),
                                );
                            }

                            for arg in &trusted_proxy_entry.args {
                                if !matches!(
                                    arg,
                                    ServerConfigurationValue::String(_, _)
                                        | ServerConfigurationValue::InterpolatedString(_, _)
                                ) {
                                    return Err(
                                        "Invalid directive 'trusted_proxy': argument type mismatch"
                                            .into(),
                                    );
                                }

                                let expanded = match arg.as_string_with_interpolations(&HashMap::<
                                    String,
                                    String,
                                >::new(
                                )) {
                                    Some(value) => value,
                                    None => {
                                        return Err(
                                            "Invalid directive 'trusted_proxy': argument type mismatch"
                                                .into(),
                                        );
                                    }
                                };
                                if expanded.parse::<IpCidr>().is_err() {
                                    return Err(format!(
                                        "Invalid directive 'trusted_proxy': '{expanded}' is not a valid IP or CIDR"
                                    )
                                    .into());
                                }
                            }
                        }
                    }
                }
            }
        }

        // Conditional directives
        if config.has_directive("if") {
            used_directives.insert("if".to_string());
        }
        if config.has_directive("if_not") {
            used_directives.insert("if_not".to_string());
        }
        if config.has_directive("location") {
            used_directives.insert("location".to_string());
        }
        if config.has_directive("handle_error") {
            used_directives.insert("handle_error".to_string());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::config::validator::ConfigurationValidator;
    use ferron_core::config::{
        ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue,
    };
    use std::collections::HashSet;

    fn client_ip_config(children: Option<ServerConfigurationBlock>) -> ServerConfigurationBlock {
        let mut directives = std::collections::HashMap::new();
        directives.insert(
            "client_ip_from_header".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String(
                    "x-forwarded-for".to_string(),
                    None,
                )],
                children,
                span: None,
            }],
        );

        ServerConfigurationBlock {
            directives: std::sync::Arc::new(directives),
            matchers: std::collections::HashMap::new(),
            span: None,
        }
    }

    fn trusted_proxy_block() -> ServerConfigurationBlock {
        let mut directives = std::collections::HashMap::new();
        directives.insert(
            "trusted_proxy".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String(
                    "10.0.0.0/8".to_string(),
                    None,
                )],
                children: None,
                span: None,
            }],
        );

        ServerConfigurationBlock {
            directives: std::sync::Arc::new(directives),
            matchers: std::collections::HashMap::new(),
            span: None,
        }
    }

    #[test]
    fn validates_client_ip_from_header_with_trusted_proxy_allowlist() {
        let validator = HttpConfigurationValidator;
        let config = client_ip_config(Some(trusted_proxy_block()));
        let mut used_directives = HashSet::new();

        validator
            .validate_block(&config, &mut used_directives, true)
            .expect("valid config should pass");

        assert!(used_directives.contains("client_ip_from_header"));
        assert!(used_directives.contains("trusted_proxy"));
    }

    #[test]
    fn rejects_unknown_nested_directive_under_client_ip_from_header() {
        let mut children_directives = std::collections::HashMap::new();
        children_directives.insert(
            "bogus".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String(
                    "10.0.0.0/8".to_string(),
                    None,
                )],
                children: None,
                span: None,
            }],
        );

        let config = client_ip_config(Some(ServerConfigurationBlock {
            directives: std::sync::Arc::new(children_directives),
            matchers: std::collections::HashMap::new(),
            span: None,
        }));

        let validator = HttpConfigurationValidator;
        let mut used_directives = HashSet::new();
        assert!(validator
            .validate_block(&config, &mut used_directives, true)
            .is_err());
    }
}
