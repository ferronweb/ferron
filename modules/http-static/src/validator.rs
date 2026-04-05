//! Configuration validator for the HTTP static file module

use ferron_core::config::ServerConfigurationValue;
use ferron_core::validate_directive;

pub struct HttpStaticConfigurationValidator;

impl ferron_core::config::validator::ConfigurationValidator for HttpStaticConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        used_directives: &mut std::collections::HashSet<String>,
        _is_global: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Static file compression (on-the-fly)
        validate_directive!(config, used_directives, compressed, optional
            args(1) => [ServerConfigurationValue::Boolean(_, _)], {});

        // Precompressed file serving
        validate_directive!(config, used_directives, precompressed, optional
            args(1) => [ServerConfigurationValue::Boolean(_, _)], {});

        // ETag generation
        validate_directive!(config, used_directives, etag, optional
            args(1) => [ServerConfigurationValue::Boolean(_, _)], {});

        // Directory listing
        validate_directive!(config, used_directives, directory_listing, optional
            args(1) => [ServerConfigurationValue::Boolean(_, _)], {});

        // Cache-Control header for static files
        validate_directive!(config, used_directives, file_cache_control, optional
            args(1) => [
                ServerConfigurationValue::String(_, _)
                    | ServerConfigurationValue::InterpolatedString(_, _)
                    | ServerConfigurationValue::Boolean(false, _)
            ], {});

        // Custom MIME type mappings
        validate_directive!(config, used_directives, mime_type, optional
            args(2) => [
                ServerConfigurationValue::String(_, _),
                ServerConfigurationValue::String(_, _)
            ], {});

        Ok(())
    }
}
