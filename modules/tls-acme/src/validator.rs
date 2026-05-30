use ferron_core::{
    config::{
        validator::{validate_scoped_block, ConfigurationValidator, ConfigurationValidatorContext},
        ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationSpan,
        ServerConfigurationValue,
    },
    validate_directive,
};
use ferron_tls::validate_tls_common;

pub struct TlsAcmeConfigurationValidator;

impl ConfigurationValidator for TlsAcmeConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        validator_ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        validate_tls_common!(config, validator_ctx);

        validate_directive!(config, validator_ctx.used_directives, challenge, optional args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)], {});
        validate_directive!(config, validator_ctx.used_directives, contact, optional args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)], {});
        validate_directive!(config, validator_ctx.used_directives, directory, optional args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)], {});
        validate_directive!(config, validator_ctx.used_directives, profile, optional args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)], {});
        validate_directive!(config, validator_ctx.used_directives, eab, optional args(2) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _), ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)], {});
        validate_directive!(config, validator_ctx.used_directives, cache, optional args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)], {});
        validate_directive!(config, validator_ctx.used_directives, save, optional args(*) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)], {});
        validate_directive!(config, validator_ctx.used_directives, post_obtain_command, optional args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)], {});
        validate_directive!(config, validator_ctx.used_directives, no_verification, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)], {});

        // Automatic TLS on demand
        validate_directive!(config, validator_ctx.used_directives, on_demand, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)], {});
        validate_directive!(config, validator_ctx.used_directives, on_demand_ask, optional args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)], {});
        validate_directive!(config, validator_ctx.used_directives, on_demand_ask_no_verification, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)], {});

        // DNS
        validate_directive!(config, validator_ctx.used_directives, dns, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)], {
            validate_scoped_block(dns, validator_ctx, "provider", "dns", None)?;
        });

        add_acme_best_practice_diagnostics(config, validator_ctx);

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

fn has_non_empty_string_value(config: &ServerConfigurationBlock, name: &str) -> bool {
    config.get_value(name).is_some_and(|value| {
        value
            .as_string_with_interpolations(&std::collections::HashMap::new())
            .is_some_and(|value| !value.trim().is_empty())
    })
}

fn add_acme_best_practice_diagnostics(
    config: &ServerConfigurationBlock,
    ctx: &mut ConfigurationValidatorContext,
) {
    if flag_enabled(config, "no_verification") {
        ctx.add_best_practice_violation(
            "`no_verification` disables TLS certificate verification for the ACME directory; use it only for testing or tightly controlled internal ACME services",
            directive_span(config, "no_verification"),
        );
    }

    if flag_enabled(config, "on_demand") && !has_non_empty_string_value(config, "on_demand_ask") {
        ctx.add_best_practice_violation(
            "`on_demand` is enabled without `on_demand_ask`; configure an approval endpoint to prevent certificate issuance for arbitrary hostnames",
            directive_span(config, "on_demand"),
        );
    }

    if flag_enabled(config, "on_demand_ask_no_verification") {
        ctx.add_best_practice_violation(
            "`on_demand_ask_no_verification` disables TLS verification for the approval endpoint; keep verification enabled unless the endpoint is strictly internal and otherwise authenticated",
            directive_span(config, "on_demand_ask_no_verification"),
        );
    }

    if !has_non_empty_string_value(config, "contact") {
        ctx.add_best_practice_violation(
            "`contact` is not configured; set an ACME account email so the certificate authority can send expiry or account notices",
            config.span.clone(),
        );
    }
}
