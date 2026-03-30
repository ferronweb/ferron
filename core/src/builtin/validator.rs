use crate::config::ServerConfigurationValue;

pub struct BuiltinGlobalConfigurationValidator;

impl crate::config::validator::ConfigurationValidator for BuiltinGlobalConfigurationValidator {
    fn validate_block(
        &self,
        config: &crate::config::ServerConfigurationBlock,
        used_directives: &mut std::collections::HashSet<String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // TODO: macro for configuration validation to reduce boilerplate
        if let Some(runtime) = config.directives.get("runtime") {
            used_directives.insert("runtime".to_string());
            for runtime in runtime {
                if runtime.args.len() != 0 {
                    return Err(format!(
                        "Invalid directive 'runtime': expected 0 arguments, got {}",
                        runtime.args.len()
                    )
                    .into());
                }
                let runtime_inner = runtime
                    .children
                    .as_ref()
                    .ok_or("Invalid directive 'runtime': missing block for 'runtime' directive")?;
                if let Some(io_uring_directive) = runtime_inner.directives.get("io_uring") {
                    if io_uring_directive.iter().any(|d| d.args.len() != 1) {
                        return Err(format!(
                            "Invalid directive 'runtime': expected 1 argument in 'io_uring' subdirective, got {}",
                            io_uring_directive
                                .iter()
                                .map(|d| d.args.len())
                                .max()
                                .unwrap_or(0)
                        )
                        .into());
                    } else if io_uring_directive
                        .iter()
                        .any(|d| !matches!(d.args[0], ServerConfigurationValue::Boolean(_, _)))
                    {
                        return Err(format!(
                            "Invalid directive 'runtime': the value for 'io_uring' subdirective isn't boolean",
                        )
                        .into());
                    }
                }
            }
        }

        Ok(())
    }
}
