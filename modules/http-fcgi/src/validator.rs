use ferron_core::config::ServerConfigurationValue;

pub struct FcgiConfigurationValidator;

impl ferron_core::config::validator::ConfigurationValidator for FcgiConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        used_directives: &mut std::collections::HashSet<String>,
        is_global: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if is_global {
            // Manual validation for fcgi_concurrent_conns directive
            if let Some(directives) = config.directives.get(stringify!(fcgi_concurrent_conns)) {
                for directive in directives {
                    if directive.args.len() != 1 {
                        return Err(format!(
                            "Invalid directive '{}': expected {} argument(s), got {}",
                            stringify!(fcgi_concurrent_conns),
                            1,
                            directive.args.len()
                        )
                        .into());
                    }
                    if !matches!(directive.args[0], ServerConfigurationValue::Number(n,_) if n > 0)
                        && !matches!(
                            directive.args[0],
                            ServerConfigurationValue::Boolean(false, _)
                        )
                    {
                        return Err(format!(
                            "Invalid directive '{}': invalid type",
                            stringify!(fcgi_concurrent_conns)
                        )
                        .into());
                    }
                }
            };
        }

        ferron_core::validate_directive!(config, used_directives, fcgi, optional args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::Boolean(_, _)], {
            ferron_core::validate_nested!(fcgi, backend, args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)]);
            ferron_core::validate_nested!(fcgi, extension, args(*) => [ServerConfigurationValue::String(_, _)]);
            ferron_core::validate_nested!(fcgi, environment, args(2) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _), ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)]);
            ferron_core::validate_nested!(fcgi, pass, optional args(1) => [ServerConfigurationValue::Boolean(_, _)]);
            ferron_core::validate_nested!(fcgi, keepalive, optional args(1) => [ServerConfigurationValue::Boolean(_, _)]);
        });

        // Alias for PHP-FPM
        ferron_core::validate_directive!(config, used_directives, fcgi_php, args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::Boolean(false, _)], {});

        Ok(())
    }
}
