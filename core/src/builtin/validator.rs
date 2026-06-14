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
