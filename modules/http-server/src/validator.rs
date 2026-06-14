use std::collections::HashMap;

use cidr::IpCidr;
use ferron_core::builtin::validate_observability_directives;
use ferron_core::config::validator::{validate_scoped_block, ConfigurationValidatorContext};
use ferron_core::config::{
    ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationSpan,
    ServerConfigurationValue,
};
use ferron_core::{check_unused_subdirectives, validate_directive, validate_nested};

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

            if config
                .get_value("default_http_port")
                .and_then(ServerConfigurationValue::as_boolean)
                == Some(false)
                && config
                    .get_value("default_https_port")
                    .and_then(ServerConfigurationValue::as_boolean)
                    == Some(false)
            {
                ctx.add_best_practice_violation(
                    "`default_http_port false` and `default_https_port false` disable implicit listeners for host blocks without explicit ports",
                    config.span.clone(),
                );
            }
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

            // Force trace toggle
            validate_nested!(http, used(sub), force_trace, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);

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
                validate_nested!(trace, used(trace_sub), trust_request, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);
                check_unused_subdirectives!(
                    trace,
                    trace_sub,
                    &mut ctx.diagnostics,
                    ctx.scope.clone()
                );
            });

            // Trace sampling
            validate_nested!(http, used(sub), trace_sampling, {
                if let Some(entries) = http.directives.get("trace_sampling") {
                    for entry in entries {
                        ferron_observability::sampler::validate_trace_sampling_directive(
                            entry, ctx,
                        )?;
                    }
                }
            });

            add_http_block_best_practice_diagnostics(http, ctx);
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
        if first_flag(config, "protocol_proxy") == Some(true) {
            ctx.add_best_practice_violation(
                "`protocol_proxy` trusts client-provided PROXY protocol addresses; enable it only on listeners reachable exclusively by trusted load balancers",
                first_entry_span(config, "protocol_proxy"),
            );
        }

        // Observability directives
        validate_observability_directives(config, ctx)?;

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
                        let mut has_trusted_proxy = false;
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
                                has_trusted_proxy = true;
                                if expanded == "0.0.0.0/0" || expanded == "::/0" {
                                    ctx.add_best_practice_violation(
                                        "`trusted_proxy` trusts every source address; restrict forwarded client IP headers to reverse proxies you control",
                                        trusted_proxy_entry.span.clone(),
                                    );
                                }
                            }
                        }
                        if !has_trusted_proxy {
                            ctx.add_best_practice_violation(
                                "`client_ip_from_header` has no trusted proxy ranges, so forwarded client IP headers will be ignored",
                                entry.span.clone(),
                            );
                        }
                    } else {
                        ctx.add_best_practice_violation(
                            "`client_ip_from_header` should include `trusted_proxy` ranges for the reverse proxies allowed to supply client IP headers",
                            entry.span.clone(),
                        );
                    }
                } else {
                    ctx.add_best_practice_violation(
                        "`client_ip_from_header` should include a nested `trusted_proxy` block so untrusted clients cannot spoof forwarded client IP headers",
                        entry.span.clone(),
                    );
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

        // HTTP-only deployment check (per-host only)
        if !is_global {
            add_http_only_best_practice_diagnostics(config, ctx);
        }

        Ok(())
    }
}

fn entry_span(entry: &ServerConfigurationDirectiveEntry) -> Option<ServerConfigurationSpan> {
    entry.span.clone().or_else(|| {
        entry.args.first().and_then(|value| match value {
            ServerConfigurationValue::String(_, span)
            | ServerConfigurationValue::Number(_, span)
            | ServerConfigurationValue::Float(_, span)
            | ServerConfigurationValue::Boolean(_, span)
            | ServerConfigurationValue::InterpolatedString(_, span) => span.clone(),
        })
    })
}

fn first_entry_span(
    block: &ServerConfigurationBlock,
    directive: &str,
) -> Option<ServerConfigurationSpan> {
    block
        .directives
        .get(directive)
        .and_then(|entries| entries.first())
        .and_then(entry_span)
}

fn first_flag(block: &ServerConfigurationBlock, directive: &str) -> Option<bool> {
    block
        .directives
        .get(directive)
        .and_then(|entries| entries.first())
        .map(ServerConfigurationDirectiveEntry::get_flag)
}

fn add_http_block_best_practice_diagnostics(
    http: &ServerConfigurationBlock,
    ctx: &mut ConfigurationValidatorContext,
) {
    if first_flag(http, "url_sanitize") == Some(false) {
        ctx.add_best_practice_violation(
            "`url_sanitize false` disables request path traversal normalization before routing; keep URL sanitization enabled unless a specific backend requires raw paths",
            first_entry_span(http, "url_sanitize"),
        );
    }

    if first_flag(http, "url_reject_backslash") == Some(false) {
        ctx.add_best_practice_violation(
            "`url_reject_backslash false` permits backslashes in request paths; keep rejection enabled to avoid backend path interpretation issues",
            first_entry_span(http, "url_reject_backslash"),
        );
    }

    if first_flag(http, "timeout") == Some(false) {
        ctx.add_best_practice_violation(
            "`timeout false` disables request pipeline timeouts; configure a bounded timeout to reduce slow request resource exhaustion",
            first_entry_span(http, "timeout"),
        );
    }

    if let Some(entries) = http.directives.get("options_allowed_methods") {
        for entry in entries {
            if let Some(methods) = entry
                .args
                .first()
                .and_then(|value| value.as_string_with_interpolations(&HashMap::new()))
            {
                let has_sensitive_method = methods.split(',').map(str::trim).any(|method| {
                    method.eq_ignore_ascii_case("TRACE") || method.eq_ignore_ascii_case("CONNECT")
                });
                if has_sensitive_method {
                    ctx.add_best_practice_violation(
                        "`options_allowed_methods` advertises TRACE or CONNECT; avoid exposing methods you do not intentionally support",
                        entry_span(entry),
                    );
                }
            }
        }
    }

    if let Some(entries) = http.directives.get("protocols") {
        for entry in entries {
            if entry
                .args
                .iter()
                .any(|value| value.as_str().is_some_and(|protocol| protocol == "h3"))
            {
                ctx.add_best_practice_violation(
                    "`protocols` enables experimental HTTP/3; verify client compatibility and operational monitoring before using it in production",
                    entry_span(entry),
                );
            }
        }
    }

    // Detect "location" block duplicates
    let mut unique_pathnames = std::collections::HashSet::new();

    for entry in http.directives.get("location").unwrap_or(&vec![]) {
        if let Some(pathname) = entry.args.first().and_then(|value| value.as_str()) {
            if unique_pathnames.contains(pathname) {
                // Duplicate pathname!
                ctx.add_best_practice_violation(
                    format!("`location` block with duplicate pathname: {pathname}"),
                    entry_span(entry),
                );
            } else {
                unique_pathnames.insert(pathname.to_string());
            }
        }
    }
}

/// Emit a best-practice violation when a non-localhost host block has no TLS
/// configuration, reminding the operator to ensure TLS termination happens
/// somewhere in the request path.
fn add_http_only_best_practice_diagnostics(
    config: &ServerConfigurationBlock,
    ctx: &mut ConfigurationValidatorContext,
) {
    if config.directives.contains_key("tls") {
        return;
    }

    if let Some(scope) = &ctx.scope {
        if let Some(hostname) = extract_hostname_from_scope(scope) {
            if !is_localhost_hostname(&hostname) {
                ctx.add_best_practice_violation(
                    "HTTP-only deployment detected. Ensure TLS termination is performed by an upstream proxy or load balancer.",
                    config.span.clone(),
                );
            }
        }
    }
}

/// Extract hostname from a scope string like "http port 80 host example.com".
fn extract_hostname_from_scope(scope: &str) -> Option<String> {
    let rest = scope.strip_prefix("http ")?;
    let after_port = rest.split_whitespace().nth(1)?;
    let hostname = after_port.strip_prefix("host ")?;
    Some(hostname.split_whitespace().next()?.to_string())
}

/// Check if a hostname refers to a loopback or otherwise local address.
fn is_localhost_hostname(hostname: &str) -> bool {
    let lower = hostname.to_ascii_lowercase();
    lower == "localhost"
        || lower == "127.0.0.1"
        || lower == "::1"
        || lower == "[::1]"
        || lower.starts_with("127.")
        || lower.ends_with(".localhost")
}
