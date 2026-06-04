use ferron_core::config::validator::ConfigurationValidator;
use ferron_tls::validate_tls_common;

pub struct TlsLocalConfigurationValidator;

impl ConfigurationValidator for TlsLocalConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        validator_ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        validate_tls_common!(config, validator_ctx);

        if !scope_looks_loopback(validator_ctx.scope.as_deref()) {
            validator_ctx.add_best_practice_violation(
                "`provider local` is intended for loopback development and testing; use ACME or manual certificates for production hostnames",
                config.span.clone(),
            );
        }

        Ok(())
    }
}

fn scope_looks_loopback(scope: Option<&str>) -> bool {
    let Some(scope) = scope else {
        return true;
    };

    scope.contains("host localhost")
        || scope.contains("host 127.0.0.1")
        || scope.contains("host ::1")
        || scope.contains("ip 127.")
        || scope.contains("ip ::1")
}
