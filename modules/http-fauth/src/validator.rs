use ferron_core::{
    config::{validator::ConfigurationValidator, ServerConfigurationValue},
    validate_directive, validate_nested,
};

pub struct ForwardedAuthenticationConfigurationValidator;

impl ConfigurationValidator for ForwardedAuthenticationConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        used_directives: &mut std::collections::HashSet<String>,
        is_global: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if is_global {
            // Manual validation for auth_to_concurrent_conns directive
            if let Some(directives) = config.directives.get(stringify!(auth_to_concurrent_conns)) {
                for directive in directives {
                    if directive.args.len() != 1 {
                        return Err(format!(
                            "Invalid directive '{}': expected {} argument(s), got {}",
                            stringify!(auth_to_concurrent_conns),
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
                            stringify!(auth_to_concurrent_conns)
                        )
                        .into());
                    }
                }
            };
        }

        validate_directive!(config, used_directives, auth_to, optional args(1) => [ServerConfigurationValue::Boolean(_, _) | ServerConfigurationValue::String(_, _)], {
            validate_nested!(auth_to, backend, args(1) => [ServerConfigurationValue::String(_, _)]);
            validate_nested!(auth_to, unix, args(1) => [ServerConfigurationValue::String(_, _)]);
            validate_nested!(auth_to, limit, args(1) => [ServerConfigurationValue::Number(_, _) | ServerConfigurationValue::Boolean(false, _)]);
            validate_nested!(auth_to, idle_timeout, args(1) => [ServerConfigurationValue::Number(_, _) | ServerConfigurationValue::String(_, _) | ServerConfigurationValue::Boolean(false, _)]);
            validate_nested!(auth_to, no_verification, optional args(1) => [ServerConfigurationValue::Boolean(_, _)]);
            validate_nested!(auth_to, copy, args(*) => [ServerConfigurationValue::String(_, _)]);
        });

        Ok(())
    }
}
