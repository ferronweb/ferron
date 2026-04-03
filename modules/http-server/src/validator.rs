use ferron_core::{config::ServerConfigurationValue, validate_directive, validate_nested};

pub struct HttpConfigurationValidator;

impl ferron_core::config::validator::ConfigurationValidator for HttpConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        used_directives: &mut std::collections::HashSet<String>,
        _is_global: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // TLS settings
        validate_directive!(config, used_directives, tls, optional
            args(1) => [ServerConfigurationValue::Boolean(_, _)]
            | args(2) => [
                ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _),
                ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)
            ],
            {
            validate_nested!(tls, provider, optional args(1) => [ServerConfigurationValue::String(_, _)]);
        });

        // HTTP settings
        validate_directive!(config, used_directives, http, no_args, {
            validate_nested!(http, protocols, args(*) => [ServerConfigurationValue::String(_, _)]);

            // HTTP/1.x settings
            validate_nested!(http, h1_enable_early_hints, optional args(1) => [ServerConfigurationValue::Boolean(_, _)]);

            // HTTP/2 settings
            validate_nested!(http, h2_initial_window_size, args(1) => [ServerConfigurationValue::Number(_, _)]);
            validate_nested!(http, h2_max_frame_size, args(1) => [ServerConfigurationValue::Number(_, _)]);
            validate_nested!(http, h2_max_concurrent_streams, args(1) => [ServerConfigurationValue::Number(_, _)]);
            validate_nested!(http, h2_max_header_list_size, args(1) => [ServerConfigurationValue::Number(_, _)]);
            validate_nested!(http, h2_enable_connect_protocol, optional args(1) => [ServerConfigurationValue::Boolean(_, _)]);
        });

        // Webroot
        validate_directive!(config, used_directives, root, args(1) => [
            ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)
        ], {});

        Ok(())
    }
}
