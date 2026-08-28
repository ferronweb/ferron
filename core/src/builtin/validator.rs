use crate::config::validator::{validate_scoped_block, ConfigurationValidationError};
use crate::config::{
    ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationSpan,
    ServerConfigurationValue,
};
use crate::{validate_directive, validate_nested};

pub struct BuiltinConfigurationValidator;

impl crate::config::validator::ConfigurationValidator for BuiltinConfigurationValidator {
    fn validate_block(
        &self,
        config: &crate::config::ServerConfigurationBlock,
        ctx: &mut crate::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ConfigurationValidationError> {
        let is_global = ctx.is_global;
        let used_directives = &mut ctx.used_directives;

        if is_global {
            validate_directive!(config, used_directives, runtime, no_args, {
                let mut sub = std::collections::HashSet::new();
                validate_nested!(runtime, used(sub), io_uring, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);
                crate::check_unused_subdirectives!(
                    runtime,
                    sub,
                    &mut ctx.diagnostics,
                    ctx.scope.clone()
                );
            });

            validate_directive!(config, used_directives, tcp, no_args, {
                let mut sub = std::collections::HashSet::new();
                validate_nested!(tcp, used(sub), listen, args(*) => [ServerConfigurationValue::String(_, _)]);
                validate_nested!(tcp, used(sub), send_buf, args(1) => [ServerConfigurationValue::Number(_, _)]);
                validate_nested!(tcp, used(sub), recv_buf, args(1) => [ServerConfigurationValue::Number(_, _)]);
                validate_nested!(tcp, used(sub), backlog, args(1) => [ServerConfigurationValue::Number(_, _)]);
                validate_nested!(tcp, used(sub), multipath, args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);
                crate::check_unused_subdirectives!(
                    tcp,
                    sub,
                    &mut ctx.diagnostics,
                    ctx.scope.clone()
                );
            });

            #[cfg(unix)]
            {
                validate_unix_directives(config, ctx)?;
            }
            #[cfg(not(unix))]
            {
                if let Some(unix_entry) = config.directives.get("unix").and_then(|d| d.first()) {
                    return Err(ConfigurationValidationError::from(
                        "Unix socket listeners cannot be used with non-Unix systems.",
                    )
                    .with_span(unix_entry.span.clone()));
                }
            }
        }

        validate_observability_directives(config, ctx)?;

        validate_control_plane_directives(config, ctx);

        Ok(())
    }
}

/// Validate the `control_plane` directive, which carries control plane metadata
/// (e.g. Kubernetes resource version, cluster name) and static OpenTelemetry
/// span links for cross-plane traceability.
fn validate_control_plane_directives(
    config: &crate::config::ServerConfigurationBlock,
    ctx: &mut crate::config::validator::ConfigurationValidatorContext,
) {
    use crate::config::ServerConfigurationValue;

    validate_directive!(config, ctx.used_directives, control_plane, optional, {
        let control_plane = match control_plane {
            Some(cp) => cp,
            None => return,
        };
        let mut sub = std::collections::HashSet::new();
        validate_nested!(control_plane, used(sub), metadata, optional);
        validate_nested!(control_plane, used(sub), span_links, optional);
        crate::check_unused_subdirectives!(
            control_plane,
            sub,
            &mut ctx.diagnostics,
            ctx.scope.clone()
        );
        // The `metadata` block holds arbitrary `key "value"` directives defined
        // by the control plane. Each directive is accepted as-is; we do not
        // constrain the key names or require them to be pre-registered.
        if let Some(metadata_entries) = control_plane.directives.get("metadata") {
            if let Some(metadata_entry) = metadata_entries.first() {
                if let Some(metadata_children) = metadata_entry.children.as_ref() {
                    for (key, entries) in metadata_children.directives.iter() {
                        if let Some(entry) = entries.first() {
                            if !matches!(
                                entry.args.first(),
                                Some(ServerConfigurationValue::String(_, _))
                                    | Some(ServerConfigurationValue::InterpolatedString(_, _))
                            ) {
                                ctx.diagnostics.push(ctx.create_diagnostic(
                                    crate::config::validator::ConfigurationValidatorDiagnosticKind::InvalidConfiguration,
                                    format!("`{key}` in `control_plane.metadata` must have a single string value"),
                                    entry_span(entry),
                                ));
                            }
                        }
                    }
                }
            }
        }
        if let Some(span_links_entries) = control_plane.directives.get("span_links") {
            for link_entry in span_links_entries {
                if let Some(link_children) = link_entry.children.as_ref() {
                    let mut link_sub = std::collections::HashSet::new();
                    validate_nested!(link_children, used(link_sub), trace_id, optional);
                    validate_nested!(link_children, used(link_sub), span_id, optional);
                    validate_nested!(link_children, used(link_sub), sampled, optional);
                    validate_nested!(link_children, used(link_sub), attributes, optional);
                    crate::check_unused_subdirectives!(
                        link_children,
                        link_sub,
                        &mut ctx.diagnostics,
                        ctx.scope.clone()
                    );
                    // Validate trace_id format (32 hex chars)
                    if let Some(tid_d) = link_children.directives.get("trace_id") {
                        for tid_e in tid_d {
                            let Some(tid_val) = tid_e.get_value() else {
                                continue;
                            };
                            if let Some(tid) = tid_val.as_str() {
                                if tid.len() != 32 || !tid.chars().all(|c| c.is_ascii_hexdigit()) {
                                    ctx.diagnostics.push(ctx.create_diagnostic(
                                    crate::config::validator::ConfigurationValidatorDiagnosticKind::InvalidConfiguration,
                                    format!("`trace_id` in `control_plane.span_links` must be exactly 32 hex characters, got `{tid}`"),
                                    entry_span(tid_e),
                                ));
                                }
                            } else {
                                ctx.diagnostics.push(ctx.create_diagnostic(
                                crate::config::validator::ConfigurationValidatorDiagnosticKind::InvalidConfiguration,
                                "`trace_id` in `control_plane.span_links` must be a string".to_string(),
                                entry_span(tid_e),
                            ));
                            }
                        }
                    }
                    if let Some(sid_d) = link_children.directives.get("span_id") {
                        for sid_e in sid_d {
                            let Some(sid_val) = sid_e.get_value() else {
                                continue;
                            };
                            if let Some(sid) = sid_val.as_str() {
                                if sid.len() != 16 || !sid.chars().all(|c| c.is_ascii_hexdigit()) {
                                    ctx.diagnostics.push(ctx.create_diagnostic(
                                    crate::config::validator::ConfigurationValidatorDiagnosticKind::InvalidConfiguration,
                                    format!("`span_id` in `control_plane.span_links` must be exactly 16 hex characters, got `{sid}`"),
                                    entry_span(sid_e),
                                ));
                                }
                            } else {
                                ctx.diagnostics.push(ctx.create_diagnostic(
                                crate::config::validator::ConfigurationValidatorDiagnosticKind::InvalidConfiguration,
                                "`span_id` in `control_plane.span_links` must be a string".to_string(),
                                entry_span(sid_e),
                            ));
                            }
                        }
                    }
                    if let Some(sampled_entries) = link_children.directives.get("sampled") {
                        if let Some(sampled_entry) = sampled_entries.first() {
                            if !matches!(
                                sampled_entry.args.first(),
                                Some(ServerConfigurationValue::Boolean(_, _))
                            ) {
                                ctx.diagnostics.push(ctx.create_diagnostic(
                                    crate::config::validator::ConfigurationValidatorDiagnosticKind::InvalidConfiguration,
                                    "`sampled` in `control_plane.span_links` must be a boolean".to_string(),
                                    entry_span(sampled_entry),
                                ));
                            }
                        }
                    }
                }
            }
        }
    });
}

pub fn validate_observability_directives(
    config: &crate::config::ServerConfigurationBlock,
    ctx: &mut crate::config::validator::ConfigurationValidatorContext,
) -> Result<(), ConfigurationValidationError> {
    // Observability settings
    validate_directive!(config, ctx.used_directives, observability, optional
        args(1) => [ServerConfigurationValue::Boolean(_, _)],
        {
            validate_scoped_block(observability, ctx, "provider", "observability", None)?;
    });

    // Alias: log /path/to/access.log { ... } -> observability { provider file; access_log /path/to/access.log; ... }
    validate_directive!(config, ctx.used_directives, log, optional
        args(1) => [ServerConfigurationValue::Boolean(_, _)]
        | args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)],
        {
            let mut block = log.clone();
            let mut directives = (*block.directives).clone();
            directives.insert("provider".to_string(), vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String("file".to_string(), None)],
                children: None,
                span: None,
            }]);
            // Placeholder log file name...
            directives.insert("access_log".to_string(), vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String("placeholder.log".to_string(), None)],
                children: None,
                span: None,
            }]);
            block.directives = std::sync::Arc::new(directives);
            validate_scoped_block(&block, ctx, "provider", "observability", None)?;
    });

    // Alias: error_log /path/to/error.log { ... } -> observability { provider file; error_log /path/to/error.log; ... }
    validate_directive!(config, ctx.used_directives, error_log, optional
        args(1) => [ServerConfigurationValue::Boolean(_, _)]
        | args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)],
        {
            let mut block = error_log.clone();
            let mut directives = (*block.directives).clone();
            directives.insert("provider".to_string(), vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String("file".to_string(), None)],
                children: None,
                span: None,
            }]);
            // Placeholder log file name...
            directives.insert("error_log".to_string(), vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String("placeholder.log".to_string(), None)],
                children: None,
                span: None,
            }]);
            block.directives = std::sync::Arc::new(directives);
            validate_scoped_block(&block, ctx, "provider", "observability", None)?;
    });

    // Alias: console_log { ... } -> observability { provider console; ... }
    validate_directive!(config, ctx.used_directives, console_log, optional
        args(1) => [ServerConfigurationValue::Boolean(_, _)],
        {
            let mut block = console_log.clone();
            let mut directives = (*block.directives).clone();
            directives.insert("provider".to_string(), vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String("console".to_string(), None)],
                children: None,
                span: None,
            }]);
            block.directives = std::sync::Arc::new(directives);
            validate_scoped_block(&block, ctx, "provider", "observability", None)?;
    });

    add_log_rotation_best_practice_diagnostics(config, ctx);

    Ok(())
}

fn add_log_rotation_best_practice_diagnostics(
    config: &ServerConfigurationBlock,
    ctx: &mut crate::config::validator::ConfigurationValidatorContext,
) {
    if let Some(entries) = config.directives.get("log") {
        for entry in entries {
            if directive_has_path_arg(entry)
                && !entry
                    .children
                    .as_ref()
                    .is_some_and(|block| block.directives.contains_key("access_log_rotate_size"))
            {
                ctx.add_best_practice_violation(
                    "`log` writes to a file without built-in rotation; configure `access_log_rotate_size` or ensure an external log rotation policy manages it",
                    entry_span(entry),
                );
            }
        }
    }

    if let Some(entries) = config.directives.get("error_log") {
        for entry in entries {
            if directive_has_path_arg(entry)
                && !entry
                    .children
                    .as_ref()
                    .is_some_and(|block| block.directives.contains_key("error_log_rotate_size"))
            {
                ctx.add_best_practice_violation(
                    "`error_log` writes to a file without built-in rotation; configure `error_log_rotate_size` or ensure an external log rotation policy manages it",
                    entry_span(entry),
                );
            }
        }
    }
}

fn directive_has_path_arg(entry: &ServerConfigurationDirectiveEntry) -> bool {
    matches!(
        entry.args.first(),
        Some(ServerConfigurationValue::String(_, _))
            | Some(ServerConfigurationValue::InterpolatedString(_, _))
    )
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

#[cfg(unix)]
fn validate_unix_directives(
    config: &ServerConfigurationBlock,
    ctx: &mut crate::config::validator::ConfigurationValidatorContext,
) -> Result<(), ConfigurationValidationError> {
    validate_directive!(config, ctx.used_directives, unix, args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)], {
        let mut sub = std::collections::HashSet::new();
        validate_nested!(unix, used(sub), backlog, optional args(1) => [ServerConfigurationValue::Number(_, _)]);
        validate_nested!(unix, used(sub), mode, optional args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _) | ServerConfigurationValue::Number(_, _)]);
        validate_nested!(unix, used(sub), owner, optional args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _) | ServerConfigurationValue::Number(_, _)]);
        validate_nested!(unix, used(sub), group, optional args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _) | ServerConfigurationValue::Number(_, _)]);
        crate::check_unused_subdirectives!(
            unix,
            sub,
            &mut ctx.diagnostics,
            ctx.scope.clone()
        );
    });

    // Validate unix paths and subdirective values (outside validate_directive! to avoid per-entry duplication)
    {
        let mut seen_unix_paths = std::collections::HashSet::new();
        for unix_entry in config.directives.get("unix").unwrap_or(&Vec::new()) {
            if let Some(val) = unix_entry.args.first() {
                if let Some(s) = val.as_str() {
                    let trimmed = s.trim();
                    if trimmed.is_empty() {
                        ctx.diagnostics.push(ctx.create_diagnostic(
                            crate::config::validator::ConfigurationValidatorDiagnosticKind::InvalidConfiguration,
                            "unix socket path cannot be empty",
                            unix_entry.span.clone(),
                        ));
                    } else if trimmed.contains('\0') {
                        ctx.diagnostics.push(ctx.create_diagnostic(
                            crate::config::validator::ConfigurationValidatorDiagnosticKind::InvalidConfiguration,
                            "unix socket path contains NUL byte",
                            unix_entry.span.clone(),
                        ));
                    } else if trimmed.len() >= 108 {
                        ctx.diagnostics.push(ctx.create_diagnostic(
                            crate::config::validator::ConfigurationValidatorDiagnosticKind::InvalidConfiguration,
                            format!("unix socket path too long ({} >= 108)", trimmed.len()),
                            unix_entry.span.clone(),
                        ));
                    } else if !seen_unix_paths.insert(trimmed.to_string()) {
                        ctx.diagnostics.push(ctx.create_diagnostic(
                            crate::config::validator::ConfigurationValidatorDiagnosticKind::InvalidConfiguration,
                            format!("duplicate unix socket path '{trimmed}'"),
                            unix_entry.span.clone(),
                        ));
                    }
                }
            }
            if let Some(children) = unix_entry.children.as_ref() {
                if let Some(mode_entries) = children.directives.get("mode") {
                    for mode_entry in mode_entries {
                        let Some(val) = mode_entry.args.first() else {
                            continue;
                        };
                        let is_valid = match val {
                            ServerConfigurationValue::InterpolatedString(_, _) => {
                                // Interpolated strings are validated at runtime, skip static check
                                true
                            }
                            ServerConfigurationValue::String(s, _) => {
                                // Octal string like "0660" or "0o660" or decimal octal digits
                                let trimmed = s.trim();
                                let octal_str =
                                    if trimmed.starts_with("0o") || trimmed.starts_with("0O") {
                                        &trimmed[2..]
                                    } else {
                                        trimmed
                                    };
                                !octal_str.is_empty()
                                    && octal_str.chars().all(|c| c.is_ascii_digit() && c <= '7')
                                    && u32::from_str_radix(octal_str, 8).is_ok_and(|v| v <= 0o777)
                            }
                            ServerConfigurationValue::Number(n, _) => *n >= 0 && *n <= 0o777,
                            _ => false,
                        };
                        if !is_valid {
                            ctx.diagnostics.push(ctx.create_diagnostic(
                                crate::config::validator::ConfigurationValidatorDiagnosticKind::InvalidConfiguration,
                                "unix `mode` must be an octal permission like \"0660\" (0..0o777) or a number 0..511",
                                mode_entry.span.clone(),
                            ));
                        }
                    }
                }
                if let Some(backlog_entries) = children.directives.get("backlog") {
                    for be in backlog_entries {
                        if let Some(ServerConfigurationValue::Number(n, _)) = be.args.first() {
                            if *n < -1 {
                                ctx.diagnostics.push(ctx.create_diagnostic(
                                    crate::config::validator::ConfigurationValidatorDiagnosticKind::InvalidConfiguration,
                                    "unix `backlog` must be >= -1",
                                    be.span.clone(),
                                ));
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}
