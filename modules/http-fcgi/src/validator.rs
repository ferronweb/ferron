use ferron_core::config::ServerConfigurationValue;

pub struct FcgiConfigurationValidator;

impl ferron_core::config::validator::ConfigurationValidator for FcgiConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let is_global = ctx.is_global;
        let used_directives = &mut ctx.used_directives;
        if is_global {
            // Manual validation for fcgi_concurrent_conns directive
            if let Some(directives) = config.directives.get(stringify!(fcgi_concurrent_conns)) {
                used_directives.insert(stringify!(fcgi_concurrent_conns).to_string());
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
            let mut sub = std::collections::HashSet::new();
            ferron_core::validate_nested!(fcgi, used(sub), backend, args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)]);
            ferron_core::validate_nested!(fcgi, used(sub), extension, args(*) => [ServerConfigurationValue::String(_, _)]);
            ferron_core::validate_nested!(fcgi, used(sub), environment, args(2) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _), ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)]);
            ferron_core::validate_nested!(fcgi, used(sub), pass, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);
            ferron_core::validate_nested!(fcgi, used(sub), keepalive, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);
            ferron_core::check_unused_subdirectives!(fcgi, sub, &mut ctx.diagnostics, ctx.scope.clone());
        });

        // Alias for PHP-FPM
        ferron_core::validate_directive!(config, used_directives, fcgi_php, args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::Boolean(false, _)], {});

        Ok(())
    }
}
