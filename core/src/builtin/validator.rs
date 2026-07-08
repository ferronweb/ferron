use crate::config::validator::validate_scoped_block;
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
    ) -> Result<(), Box<dyn std::error::Error>> {
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
                validate_nested!(tcp, used(sub), listen, args(1) => [ServerConfigurationValue::String(_, _)]);
                validate_nested!(tcp, used(sub), send_buf, args(1) => [ServerConfigurationValue::Number(_, _)]);
                validate_nested!(tcp, used(sub), recv_buf, args(1) => [ServerConfigurationValue::Number(_, _)]);
                validate_nested!(tcp, used(sub), backlog, args(1) => [ServerConfigurationValue::Number(_, _)]);
                validate_nested!(tcp, used(sub), multipath, args(1) => [ServerConfigurationValue::Boolean(_, _)]);
                crate::check_unused_subdirectives!(
                    tcp,
                    sub,
                    &mut ctx.diagnostics,
                    ctx.scope.clone()
                );
            });
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
        // Validate `span_links` sub-blocks
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
                    if let Some(tid_val) = link_children.get_value("trace_id") {
                        if let Some(tid) = tid_val.as_str() {
                            if tid.len() != 32 || !tid.chars().all(|c| c.is_ascii_hexdigit()) {
                                ctx.diagnostics.push(ctx.create_diagnostic(
                                    crate::config::validator::ConfigurationValidatorDiagnosticKind::InvalidConfiguration,
                                    format!("`trace_id` in `control_plane.span_links` must be exactly 32 hex characters, got `{tid}`"),
                                    link_children.span.clone(),
                                ));
                            }
                        } else {
                            ctx.diagnostics.push(ctx.create_diagnostic(
                                crate::config::validator::ConfigurationValidatorDiagnosticKind::InvalidConfiguration,
                                "`trace_id` in `control_plane.span_links` must be a string".to_string(),
                                link_children.span.clone(),
                            ));
                        }
                    }
                    // Validate span_id format (16 hex chars)
                    if let Some(sid_val) = link_children.get_value("span_id") {
                        if let Some(sid) = sid_val.as_str() {
                            if sid.len() != 16 || !sid.chars().all(|c| c.is_ascii_hexdigit()) {
                                ctx.diagnostics.push(ctx.create_diagnostic(
                                    crate::config::validator::ConfigurationValidatorDiagnosticKind::InvalidConfiguration,
                                    format!("`span_id` in `control_plane.span_links` must be exactly 16 hex characters, got `{sid}`"),
                                    link_children.span.clone(),
                                ));
                            }
                        } else {
                            ctx.diagnostics.push(ctx.create_diagnostic(
                                crate::config::validator::ConfigurationValidatorDiagnosticKind::InvalidConfiguration,
                                "`span_id` in `control_plane.span_links` must be a string".to_string(),
                                link_children.span.clone(),
                            ));
                        }
                    }
                    // Validate sampled is a boolean if present
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
) -> Result<(), Box<dyn std::error::Error>> {
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
