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

        Ok(())
    }
}
