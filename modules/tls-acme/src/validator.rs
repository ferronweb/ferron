use ferron_core::config::validator::{
    validate_scoped_block, ConfigurationValidator, ConfigurationValidatorContext,
};
use ferron_core::config::{
    ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationSpan,
    ServerConfigurationValue,
};
use ferron_core::validate_directive;
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
        validate_directive!(config, validator_ctx.used_directives, on_demand_ask_auth, optional args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)], {});
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

    add_non_public_domain_diagnostics(config, ctx);
}

/// Check if a domain name uses a non-public TLD that is unlikely to be resolvable
/// by ACME certificate authorities.
fn is_non_public_domain(domain: &str) -> bool {
    // 1. Strip trailing root dot and normalize to lowercase
    let normalized = domain.trim_end_matches('.').to_ascii_lowercase();

    // 2. Extract the final segment after the last dot
    let extracted_tld = normalized
        .rsplit_once('.')
        .map_or(normalized.as_str(), |(_, tld)| tld);

    // 3. Verify against the IANA-backed static map
    !tld::exist(extracted_tld)
}

/// Emit diagnostics for domains that are unlikely to be publicly resolvable,
/// which would cause ACME certificate issuance to fail.
fn add_non_public_domain_diagnostics(
    config: &ServerConfigurationBlock,
    ctx: &mut ConfigurationValidatorContext,
) {
    // Check explicit domains directive
    if let Some(entries) = config.directives.get("domains") {
        for entry in entries {
            for arg in &entry.args {
                if let Some(domain) = arg.as_str() {
                    if is_non_public_domain(domain) {
                        ctx.add_best_practice_violation(
                            format!(
                                "Domain `{domain}` is unlikely to be publicly resolvable; automatic TLS certificate issuance may fail"
                            ),
                            entry_span(entry),
                        );
                    }
                }
            }
        }
    }

    // Check scope for on_demand or implicit domain contexts
    if let Some(scope) = &ctx.scope {
        if let Some(hostname) = extract_hostname_from_scope(scope) {
            if is_non_public_domain(&hostname) {
                ctx.add_best_practice_violation(
                    format!(
                        "Domain `{hostname}` is unlikely to be publicly resolvable; automatic TLS certificate issuance may fail"
                    ),
                    config.span.clone(),
                );
            }
        }
    }
}

/// Extract hostname from a scope string like "http port 80 host example.com".
fn extract_hostname_from_scope(scope: &str) -> Option<String> {
    let (_, rest) = scope.split_once(' ')?; // for example, "http ..."

    // Skip "port "
    let after_host = rest.strip_prefix("port ")?;

    // Skip port number
    let after_port = after_host.split_whitespace().nth(1)?;
    // after_port is either "host <name>" or "ip <addr>"
    let hostname = after_port.strip_prefix("host ")?;
    // Take until the next space (there might be "ip ..." after)
    Some(hostname.split_whitespace().next()?.to_string())
}
