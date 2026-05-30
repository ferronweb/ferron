use std::collections::HashMap;

use cidr::IpCidr;
use ferron_core::{
    config::{validator::validate_scoped_block, ServerConfigurationValue},
    check_unused_subdirectives, validate_directive, validate_nested,
};

pub struct HttpConfigurationValidator;

impl ferron_core::config::validator::ConfigurationValidator for HttpConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let is_global = ctx.is_global;
        // Global-only directives (default port configuration)
        if is_global {
            validate_directive!(config, ctx.used_directives, default_http_port, optional args(1) => [
                ServerConfigurationValue::Number(_, _)
                    | ServerConfigurationValue::Boolean(_, _)
            ], {});

            validate_directive!(config, ctx.used_directives, default_https_port, optional args(1) => [
                ServerConfigurationValue::Number(_, _)
                    | ServerConfigurationValue::Boolean(_, _)
            ], {});
        }

        // TLS settings
        validate_directive!(config, ctx.used_directives, tls, optional
            args(1) => [ServerConfigurationValue::Boolean(_, _)]
            | args(2) => [
                ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _),
                ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)
            ],
            {
              validate_scoped_block(tls, ctx, "provider", "tls", Some("manual"))?;
        });

        // HTTP settings
        validate_directive!(config, ctx.used_directives, http, no_args, {
            let mut sub = std::collections::HashSet::new();

            validate_nested!(http, used(sub), protocols, args(*) => [ServerConfigurationValue::String(_, _)]);

            // OPTIONS * allowed methods
            validate_nested!(http, used(sub), options_allowed_methods, args(1) => [
                ServerConfigurationValue::String(_, _)
                    | ServerConfigurationValue::InterpolatedString(_, _)
            ]);

            // Timeout
            validate_nested!(http, used(sub), timeout, args(1) => [
                ServerConfigurationValue::Number(_, _)
                    | ServerConfigurationValue::Boolean(false, _)
                    | ServerConfigurationValue::String(_, _)
                    | ServerConfigurationValue::InterpolatedString(_, _)
            ]);

            // URL sanitization
            if is_global {
                validate_nested!(http, used(sub), url_sanitize, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);
                validate_nested!(http, used(sub), url_reject_backslash, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);
            }

            // HTTP/1.x settings
            validate_nested!(http, used(sub), h1_enable_early_hints, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);

            // 103 Early Hints
            validate_nested!(http, used(sub), early_hints, optional);

            // HTTP/2 settings
            validate_nested!(http, used(sub), h2_initial_window_size, args(1) => [ServerConfigurationValue::Number(_, _)]);
            validate_nested!(http, used(sub), h2_max_frame_size, args(1) => [ServerConfigurationValue::Number(_, _)]);
            validate_nested!(http, used(sub), h2_max_concurrent_streams, args(1) => [ServerConfigurationValue::Number(_, _)]);
            validate_nested!(http, used(sub), h2_max_header_list_size, args(1) => [ServerConfigurationValue::Number(_, _)]);
            validate_nested!(http, used(sub), h2_enable_connect_protocol, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);

            // W3C Trace Context
            validate_nested!(http, used(sub), trace, {
                let mut trace_sub = std::collections::HashSet::new();
                validate_nested!(trace, used(trace_sub), generate, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);
                validate_nested!(trace, used(trace_sub), sampled, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);
                check_unused_subdirectives!(trace, trace_sub, &mut ctx.diagnostics, ctx.scope.clone());
            });

            check_unused_subdirectives!(http, sub, &mut ctx.diagnostics, ctx.scope.clone());
        });

        // Webroot
        validate_directive!(config, ctx.used_directives, root, args(1) => [
            ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)
        ], {});

        // Server administrator's email address
        validate_directive!(config, ctx.used_directives, admin_email, args(1) => [
            ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)
        ], {});

        // PROXY protocol
        validate_directive!(config, ctx.used_directives, protocol_proxy, optional args(1) => [
            ServerConfigurationValue::Boolean(_, _)
        ], {});

        // Observability aliases
        validate_directive!(config, ctx.used_directives, log, optional
            args(1) => [ServerConfigurationValue::Boolean(_, _)]
            | args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)],
            {
            let mut sub = std::collections::HashSet::new();
            validate_nested!(log, used(sub), format, args(1) => ServerConfigurationValue::String(_, _));
            validate_nested!(log, used(sub), access_log_rotate_size, optional args(1) => [ServerConfigurationValue::Number(_, _)]);
            validate_nested!(log, used(sub), access_log_rotate_keep, optional args(1) => [ServerConfigurationValue::Number(_, _)]);
            check_unused_subdirectives!(log, sub, &mut ctx.diagnostics, ctx.scope.clone());
        });

        validate_directive!(config, ctx.used_directives, error_log, optional
            args(1) => [ServerConfigurationValue::Boolean(_, _)]
            | args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)],
            {
            let mut sub = std::collections::HashSet::new();
            validate_nested!(error_log, used(sub), error_log_rotate_size, optional args(1) => [ServerConfigurationValue::Number(_, _)]);
            validate_nested!(error_log, used(sub), error_log_rotate_keep, optional args(1) => [ServerConfigurationValue::Number(_, _)]);
            check_unused_subdirectives!(error_log, sub, &mut ctx.diagnostics, ctx.scope.clone());
        });

        validate_directive!(config, ctx.used_directives, console_log, optional
            args(1) => [ServerConfigurationValue::Boolean(_, _)],
            {
            let mut sub = std::collections::HashSet::new();
            validate_nested!(console_log, used(sub), format, args(1) => ServerConfigurationValue::String(_, _));
            check_unused_subdirectives!(console_log, sub, &mut ctx.diagnostics, ctx.scope.clone());
        });

        // Index file names
        validate_directive!(config, ctx.used_directives, index, optional args(?), {});

        // Trailing slash redirect for directories
        validate_directive!(config, ctx.used_directives, trailing_slash_redirect, optional args(1) => [
            ServerConfigurationValue::Boolean(_, _)
        ], {});

        // HTTPS redirect toggle
        validate_directive!(config, ctx.used_directives, https_redirect, optional args(1) => [
            ServerConfigurationValue::Boolean(_, _)
        ], {});

        // Client IP from forwarded header
        if let Some(entries) = config.directives.get("client_ip_from_header") {
            ctx.used_directives
                .insert("client_ip_from_header".to_string());
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
                        ctx.used_directives.insert("trusted_proxy".to_string());
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
            ctx.used_directives.insert("if".to_string());
        }
        if config.has_directive("if_not") {
            ctx.used_directives.insert("if_not".to_string());
        }
        if config.has_directive("location") {
            ctx.used_directives.insert("location".to_string());
        }
        if config.has_directive("handle_error") {
            ctx.used_directives.insert("handle_error".to_string());
        }

        Ok(())
    }
}
