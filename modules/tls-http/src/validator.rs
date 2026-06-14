use ferron_core::config::validator::{ConfigurationValidator, ConfigurationValidatorContext};
use ferron_core::config::{
    ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationSpan,
    ServerConfigurationValue,
};
use ferron_core::validate_directive;
use ferron_tls::validate_tls_common;

pub struct TlsHttpConfigurationValidator;

impl ConfigurationValidator for TlsHttpConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        validator_ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        validate_tls_common!(config, validator_ctx);

        validate_directive!(config, validator_ctx.used_directives, url, optional args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)], {});
        validate_directive!(config, validator_ctx.used_directives, refresh_interval, optional args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _) | ServerConfigurationValue::Number(_, _)], {});
        validate_directive!(config, validator_ctx.used_directives, no_verification, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)], {});

        add_http_tls_best_practice_diagnostics(config, validator_ctx);

        Ok(())
    }
}

fn directive_span(
    config: &ServerConfigurationBlock,
    name: &str,
) -> Option<ServerConfigurationSpan> {
    config
        .directives
        .get(name)
        .and_then(|entries| entries.first())
        .and_then(entry_span)
}

fn entry_span(entry: &ServerConfigurationDirectiveEntry) -> Option<ServerConfigurationSpan> {
    entry.span.clone().or_else(|| {
        entry.args.first().and_then(|value| match value {
            ServerConfigurationValue::String(_, span)
            | ServerConfigurationValue::Number(_, span)
            | ServerConfigurationValue::Float(_, span)
            | ServerConfigurationValue::Boolean(_, span)
            | ServerConfigurationValue::InterpolatedString(_, span) => span.clone(),
        })
    })
}

fn flag_enabled(config: &ServerConfigurationBlock, name: &str) -> bool {
    config.directives.get(name).is_some_and(|entries| {
        entries
            .first()
            .is_some_and(ServerConfigurationDirectiveEntry::get_flag)
    })
}

fn add_http_tls_best_practice_diagnostics(
    config: &ServerConfigurationBlock,
    ctx: &mut ConfigurationValidatorContext,
) {
    if let Some(ServerConfigurationValue::String(url, span)) = config.get_value("url") {
        if url.starts_with("http://") {
            ctx.add_best_practice_violation(
                "`url` uses plain HTTP for a certificate endpoint that returns private keys; use HTTPS with authentication in production",
                span.clone(),
            );
        }
    }

    if flag_enabled(config, "no_verification") {
        ctx.add_best_practice_violation(
            "`no_verification` disables TLS certificate verification for the certificate endpoint; keep verification enabled unless the endpoint is strictly internal and otherwise authenticated",
            directive_span(config, "no_verification"),
        );
    }
}
