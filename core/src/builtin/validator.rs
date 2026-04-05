use crate::{config::ServerConfigurationValue, validate_directive, validate_nested};

pub struct BuiltinGlobalConfigurationValidator;

impl crate::config::validator::ConfigurationValidator for BuiltinGlobalConfigurationValidator {
    fn validate_block(
        &self,
        config: &crate::config::ServerConfigurationBlock,
        used_directives: &mut std::collections::HashSet<String>,
        is_global: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if is_global {
            validate_directive!(config, used_directives, runtime, no_args, {
                validate_nested!(runtime, io_uring, optional args(1) => [ServerConfigurationValue::Boolean(_, _)]);
            });

            validate_directive!(config, used_directives, tcp, no_args, {
                validate_nested!(tcp, listen, args(1) => [ServerConfigurationValue::String(_, _)]);
                validate_nested!(tcp, send_buf, args(1) => [ServerConfigurationValue::Number(_, _)]);
                validate_nested!(tcp, recv_buf, args(1) => [ServerConfigurationValue::Number(_, _)]);
            });
        }

        // Observability settings
        validate_directive!(config, used_directives, observability, optional
            args(1) => [ServerConfigurationValue::Boolean(_, _)],
            {
            validate_nested!(observability, provider, args(1) => ServerConfigurationValue::String(_, _));

            // Common fields
            validate_nested!(observability, format, args(1) => ServerConfigurationValue::String(_, _));
        });

        // Alias: log /path/to/access.log { ... } -> observability { provider file; access_log /path/to/access.log; ... }
        validate_directive!(config, used_directives, log, optional
            args(1) => [ServerConfigurationValue::Boolean(_, _)]
            | args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)],
            {
            validate_nested!(log, format, args(1) => ServerConfigurationValue::String(_, _));
            validate_nested!(log, access_log_rotate_size, optional args(1) => [ServerConfigurationValue::Number(_, _)]);
            validate_nested!(log, access_log_rotate_keep, optional args(1) => [ServerConfigurationValue::Number(_, _)]);
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
                    if let Some(rotate_size_entries) =
                        children.directives.get("error_log_rotate_size")
                    {
                        used_directives.insert("error_log_rotate_size".to_string());
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
                    if let Some(rotate_keep_entries) =
                        children.directives.get("error_log_rotate_keep")
                    {
                        used_directives.insert("error_log_rotate_keep".to_string());
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
                }
                // error_log may or may not have children, both are valid
            }
        }

        // Alias: console_log { ... } -> observability { provider console; ... }
        validate_directive!(config, used_directives, console_log, optional
            args(1) => [ServerConfigurationValue::Boolean(_, _)],
            {
            validate_nested!(console_log, format, args(1) => ServerConfigurationValue::String(_, _));
        });

        Ok(())
    }
}
