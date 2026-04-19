use ferron_core::config::ServerConfigurationValue;

pub struct CgiConfigurationValidator;

impl ferron_core::config::validator::ConfigurationValidator for CgiConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        used_directives: &mut std::collections::HashSet<String>,
        _is_global: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        ferron_core::validate_directive!(config, used_directives, cgi, optional args(1) => [ServerConfigurationValue::Boolean(_, _)], {
            ferron_core::validate_nested!(cgi, extension, args(*) => [ServerConfigurationValue::String(_, _)]);

            // Manual validation of `interpreter` subdirective...
            if let Some(directives) = cgi.directives.get(stringify!(interpreter)) {
                for directive in directives {
                    let mut strings_only = true;
                    for arg in directive.args.iter() {
                        if !matches!(arg, ServerConfigurationValue::String(_, _)) {
                            strings_only = false;
                            break;
                        }
                    }
                    if !matches!(directive.args[0], ServerConfigurationValue::String(_, _))
                        && !strings_only
                    {
                        return Err(format!(
                            "Invalid directive '{}': invalid type for '{}' subdirective at position {}",
                            stringify!(cgi),
                            stringify!(interpreter),
                            0
                        )
                        .into());
                    }
                    if !matches!(
                        directive.args[1],
                        ServerConfigurationValue::Boolean(false, _)
                    ) && !strings_only
                    {
                        return Err(format!(
                            "Invalid directive '{}': invalid type for '{}' subdirective at position {}",
                            stringify!(cgi),
                            stringify!(interpreter),
                            1
                        )
                        .into());
                    };
                }
            };

            ferron_core::validate_nested!(cgi, environment, args(2) => [ServerConfigurationValue::String(_, _), ServerConfigurationValue::String(_, _)]);
        });

        Ok(())
    }
}
