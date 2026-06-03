use crate::{
    config::{
        ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationSpan,
        ServerConfigurationValue,
    },
    validate_directive, validate_nested,
};

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
                crate::check_unused_subdirectives!(
                    tcp,
                    sub,
                    &mut ctx.diagnostics,
                    ctx.scope.clone()
                );
            });
        }

        validate_observability_directives(config, ctx)?;

        Ok(())
    }
}

pub fn validate_observability_directives(
    config: &crate::config::ServerConfigurationBlock,
    ctx: &mut crate::config::validator::ConfigurationValidatorContext,
) -> Result<(), Box<dyn std::error::Error>> {
    let used_directives = &mut ctx.used_directives;

    // Observability settings
    validate_directive!(config, used_directives, observability, optional
        args(1) => [ServerConfigurationValue::Boolean(_, _)],
        {
        let mut sub = std::collections::HashSet::new();
        validate_nested!(observability, used(sub), provider, args(1) => ServerConfigurationValue::String(_, _));

        // Common fields
        validate_nested!(observability, used(sub), format, args(1) => ServerConfigurationValue::String(_, _));
        crate::check_unused_subdirectives!(observability, sub, &mut ctx.diagnostics, ctx.scope.clone());
    });

    // Alias: log /path/to/access.log { ... } -> observability { provider file; access_log /path/to/access.log; ... }
    validate_directive!(config, used_directives, log, optional
        args(1) => [ServerConfigurationValue::Boolean(_, _)]
        | args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)],
        {
        let mut sub = std::collections::HashSet::new();
        validate_nested!(log, used(sub), format, args(1) => ServerConfigurationValue::String(_, _));
        validate_nested!(log, used(sub), access_log_rotate_size, optional args(1) => [ServerConfigurationValue::Number(_, _)]);
        validate_nested!(log, used(sub), access_log_rotate_keep, optional args(1) => [ServerConfigurationValue::Number(_, _)]);
        crate::check_unused_subdirectives!(log, sub, &mut ctx.diagnostics, ctx.scope.clone());
    });

    // Alias: error_log /path/to/error.log { ... } -> observability { provider file; error_log /path/to/error.log; ... }
    // Note: error_log may or may not have a nested block, so we validate it manually
    if let Some(directives) = config.directives.get("error_log") {
        used_directives.insert("error_log".to_string());
        for directive in directives {
            let arg_count = directive.args.len();
            if arg_count != 1 {
                return Err(format!(
                    "Invalid directive 'error_log': expected 1 argument, got {}",
                    arg_count
                )
                .into());
            }

            let is_valid = matches!(
                directive.args.first(),
                Some(ServerConfigurationValue::Boolean(_, _))
                    | Some(ServerConfigurationValue::String(_, _))
                    | Some(ServerConfigurationValue::InterpolatedString(_, _))
            );

            if !is_valid {
                return Err("Invalid directive 'error_log': argument type mismatch".into());
            }
            // Validate nested block if present
            if let Some(ref children) = directive.children {
                let mut sub = std::collections::HashSet::new();
                if let Some(rotate_size_entries) = children.directives.get("error_log_rotate_size")
                {
                    sub.insert("error_log_rotate_size".to_string());
                    for entry in rotate_size_entries {
                        if entry.args.len() != 1 {
                            return Err(format!(
                                "Invalid directive 'error_log_rotate_size': expected 1 argument, got {}",
                                entry.args.len()
                            ).into());
                        }
                        if !matches!(
                            entry.args.first(),
                            Some(ServerConfigurationValue::Number(_, _))
                        ) {
                            return Err("Invalid directive 'error_log_rotate_size': argument must be a number".into());
                        }
                    }
                }
                if let Some(rotate_keep_entries) = children.directives.get("error_log_rotate_keep")
                {
                    sub.insert("error_log_rotate_keep".to_string());
                    for entry in rotate_keep_entries {
                        if entry.args.len() != 1 {
                            return Err(format!(
                                "Invalid directive 'error_log_rotate_keep': expected 1 argument, got {}",
                                entry.args.len()
                            ).into());
                        }
                        if !matches!(
                            entry.args.first(),
                            Some(ServerConfigurationValue::Number(_, _))
                        ) {
                            return Err("Invalid directive 'error_log_rotate_keep': argument must be a number".into());
                        }
                    }
                }
                crate::check_unused_subdirectives!(
                    children,
                    sub,
                    &mut ctx.diagnostics,
                    ctx.scope.clone()
                );
            }
            // error_log may or may not have children, both are valid
        }
    }

    // Alias: console_log { ... } -> observability { provider console; ... }
    validate_directive!(config, used_directives, console_log, optional
        args(1) => [ServerConfigurationValue::Boolean(_, _)],
        {
        let mut sub = std::collections::HashSet::new();
        validate_nested!(console_log, used(sub), format, args(1) => ServerConfigurationValue::String(_, _));
        crate::check_unused_subdirectives!(console_log, sub, &mut ctx.diagnostics, ctx.scope.clone());
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
