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

            // Session ticket keys configuration (validated at runtime by provider)
            validate_nested!(tls, ticket_keys, optional args(?) => [
                ServerConfigurationValue::String(_, _)
                    | ServerConfigurationValue::InterpolatedString(_, _)
                    | ServerConfigurationValue::Number(_, _)
                    | ServerConfigurationValue::Boolean(_, _)
            ]);
        });

        // HTTP settings
        validate_directive!(config, used_directives, http, no_args, {
            validate_nested!(http, protocols, args(*) => [ServerConfigurationValue::String(_, _)]);

            // Timeout
            validate_nested!(http, timeout, args(1) => [
                ServerConfigurationValue::Number(_, _)
                    | ServerConfigurationValue::Boolean(false, _)
                    | ServerConfigurationValue::String(_, _)
                    | ServerConfigurationValue::InterpolatedString(_, _)
            ]);

            // URL sanitization
            validate_nested!(http, url_sanitize, optional args(1) => [ServerConfigurationValue::Boolean(_, _)]);

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

        // Server administrator's email address
        validate_directive!(config, used_directives, admin_email, args(1) => [
            ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)
        ], {});

        // PROXY protocol
        validate_directive!(config, used_directives, protocol_proxy, optional args(1) => [
            ServerConfigurationValue::Boolean(_, _)
        ], {});

        // Observability aliases
        validate_directive!(config, used_directives, log, optional
            args(1) => [ServerConfigurationValue::Boolean(_, _)]
            | args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)],
            {
            validate_nested!(log, format, args(1) => ServerConfigurationValue::String(_, _));
        });

        validate_directive!(config, used_directives, error_log, optional
            args(1) => [ServerConfigurationValue::Boolean(_, _)]
            | args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)],
            {});

        validate_directive!(config, used_directives, console_log, optional
            args(1) => [ServerConfigurationValue::Boolean(_, _)],
            {
            validate_nested!(console_log, format, args(1) => ServerConfigurationValue::String(_, _));
        });

        Ok(())
    }
}
