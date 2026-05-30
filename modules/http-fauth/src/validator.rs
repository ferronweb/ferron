use ferron_core::{
    check_unused_subdirectives,
    config::{validator::ConfigurationValidator, ServerConfigurationValue},
    validate_directive, validate_nested,
};

pub struct ForwardedAuthenticationConfigurationValidator;

impl ConfigurationValidator for ForwardedAuthenticationConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let is_global = ctx.is_global;
        let used_directives = &mut ctx.used_directives;
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

        validate_directive!(config, used_directives, auth_to, optional args(1) => [ServerConfigurationValue::Boolean(_, _) | ServerConfigurationValue::InterpolatedString(_, _) | ServerConfigurationValue::String(_, _)], {
            let mut sub = std::collections::HashSet::new();
            validate_nested!(auth_to, used(sub), url, args(1) => [ServerConfigurationValue::InterpolatedString(_, _) | ServerConfigurationValue::String(_, _)]);
            validate_nested!(auth_to, used(sub), unix, args(1) => [ServerConfigurationValue::InterpolatedString(_, _) | ServerConfigurationValue::String(_, _)]);
            validate_nested!(auth_to, used(sub), limit, args(1) => [ServerConfigurationValue::Number(_, _) | ServerConfigurationValue::Boolean(false, _)]);
            validate_nested!(auth_to, used(sub), idle_timeout, args(1) => [ServerConfigurationValue::Number(_, _) | ServerConfigurationValue::String(_, _) | ServerConfigurationValue::Boolean(false, _)]);
            validate_nested!(auth_to, used(sub), no_verification, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);
            validate_nested!(auth_to, used(sub), copy, args(*) => [ServerConfigurationValue::String(_, _)]);
            check_unused_subdirectives!(auth_to, sub, &mut ctx.diagnostics, ctx.scope.clone());
        });

        Ok(())
    }
}
