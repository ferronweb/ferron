use ferron_core::config::validator::{
    first_entry_span, ConfigurationValidationError, ConfigurationValidator,
    ConfigurationValidatorContext,
};
use ferron_core::config::{
    ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationSpan,
    ServerConfigurationValue,
};

pub struct DnsStalwartConfigurationValidator;

impl ConfigurationValidator for DnsStalwartConfigurationValidator {
    fn validate_block(
        &self,
        config: &ServerConfigurationBlock,
        ctx: &mut ConfigurationValidatorContext,
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        let provider = config
            .get_value("provider")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        validate_provider(config, ctx, provider)?;

        Ok(())
    }
}

fn validate_provider(
    config: &ServerConfigurationBlock,
    ctx: &mut ConfigurationValidatorContext,
    provider: &str,
) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
    // Mark provider as used
    ctx.used_directives.insert("provider".to_string());

    match provider {
        // Simple single-credential providers
        "bunny" => {
            req_str(config, ctx, provider, "api_key")?;
        }
        "cloudflare" => {
            req_str(config, ctx, provider, "api_key")?;
        }
        "desec" => {
            req_str(config, ctx, provider, "auth_token")?;
        }
        "digitalocean" => {
            req_str(config, ctx, provider, "auth_token")?;
        }

        // Dual-credential providers
        "dnsimple" => {
            req_str(config, ctx, provider, "oauth_token")?;
            req_str(config, ctx, provider, "account_id")?;
        }

        // Triple-credential providers
        "porkbun" => {
            req_str(config, ctx, provider, "api_key")?;
            req_str(config, ctx, provider, "secret_key")?;
        }
        "spaceship" => {
            req_str(config, ctx, provider, "api_key")?;
            req_str(config, ctx, provider, "api_secret")?;
        }

        // Providers with optional directives
        "googlecloud" => {
            req_str(config, ctx, provider, "service_account_json")?;
            req_str(config, ctx, provider, "project_id")?;
            opt_str(ctx, "managed_zone");
            opt_bool(ctx, "private_zone");
            opt_str(ctx, "impersonate_service_account");
        }
        "ovh" => {
            req_str(config, ctx, provider, "application_key")?;
            req_str(config, ctx, provider, "application_secret")?;
            req_str(config, ctx, provider, "consumer_key")?;
            req_enum(
                config,
                ctx,
                provider,
                "endpoint",
                &[
                    "ovh-eu",
                    "ovh-ca",
                    "kimsufi-eu",
                    "kimsufi-ca",
                    "soyoustart-eu",
                    "soyoustart-ca",
                ],
            )?;
        }
        "route53" => {
            req_str(config, ctx, provider, "access_key_id")?;
            req_str(config, ctx, provider, "secret_access_key")?;
            opt_str(ctx, "region");
            opt_str(ctx, "session_token");
            opt_str(ctx, "hosted_zone_id");
            opt_bool(ctx, "private_zone_only");
            opt_str(ctx, "endpoint");
        }

        // Special cases
        "rfc2136" => {
            req_str(config, ctx, provider, "server")?;
            req_str(config, ctx, provider, "key_name")?;
            req_str(config, ctx, provider, "key_secret")?;
            req_enum(
                config,
                ctx,
                provider,
                "key_algorithm",
                &[
                    "HMAC-MD5",
                    "GSS",
                    "HMAC-SHA1",
                    "HMAC-SHA224",
                    "HMAC-SHA256",
                    "HMAC-SHA256-128",
                    "HMAC-SHA384",
                    "HMAC-SHA384-192",
                    "HMAC-SHA512",
                    "HMAC-SHA512-256",
                ],
            )?;
        }

        _ => {}
    }

    add_dns_secret_best_practice_diagnostics(config, ctx);

    Ok(())
}

/// Validate that a required string directive is present and is a String or InterpolatedString.
fn req_str(
    config: &ServerConfigurationBlock,
    ctx: &mut ConfigurationValidatorContext,
    provider: &str,
    key: &str,
) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
    ctx.used_directives.insert(key.to_string());
    match config.get_value(key) {
        Some(ServerConfigurationValue::String(_, _))
        | Some(ServerConfigurationValue::InterpolatedString(_, _)) => Ok(()),
        _ => Err(ConfigurationValidationError::from(format!(
            "Missing or invalid directive '{key}' for '{provider}' DNS provider"
        ))
        .with_span(first_entry_span(config, key))),
    }
}

/// Register an optional string directive (String or InterpolatedString).
fn opt_str(ctx: &mut ConfigurationValidatorContext, key: &str) {
    ctx.used_directives.insert(key.to_string());
}

/// Register an optional boolean directive.
fn opt_bool(ctx: &mut ConfigurationValidatorContext, key: &str) {
    ctx.used_directives.insert(key.to_string());
}

/// Register an optional numeric directive.
/*fn opt_num(ctx: &mut ConfigurationValidatorContext, key: &str) {
    ctx.used_directives.insert(key.to_string());
}*/

/// Validate that a required directive is present and its string value is one of the allowed values.
fn req_enum(
    config: &ServerConfigurationBlock,
    ctx: &mut ConfigurationValidatorContext,
    provider: &str,
    key: &str,
    allowed: &[&str],
) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
    ctx.used_directives.insert(key.to_string());
    let value = match config.get_value(key) {
        Some(ServerConfigurationValue::String(s, _)) => s.as_str(),
        Some(ServerConfigurationValue::InterpolatedString(_, _)) => {
            // Interpolated strings are validated at runtime
            return Ok(());
        }
        _ => {
            return Err(ConfigurationValidationError::from(format!(
                "Missing or invalid directive '{key}' for '{provider}' DNS provider"
            ))
            .with_span(first_entry_span(config, key)));
        }
    };
    if !allowed.contains(&value) {
        return Err(ConfigurationValidationError::from(format!(
            "Invalid value '{value}' for directive '{key}' in '{provider}' DNS provider: must be one of {}",
            allowed
                .iter()
                .map(|v| format!("'{v}'"))
                .collect::<Vec<_>>()
                .join(", ")
        ))
        .with_span(first_entry_span(config, key)));
    }
    Ok(())
}

fn add_dns_secret_best_practice_diagnostics(
    config: &ServerConfigurationBlock,
    ctx: &mut ConfigurationValidatorContext,
) {
    for (key, entries) in config.directives.iter() {
        if !is_sensitive_dns_key(key) {
            continue;
        }

        for entry in entries {
            if entry
                .args
                .iter()
                .any(|value| matches!(value, ServerConfigurationValue::String(_, _)))
            {
                ctx.add_best_practice_violation(
                    format!(
                        "`{key}` appears to contain a DNS provider secret directly in configuration; prefer environment variable interpolation or another secret-management path"
                    ),
                    entry_span(entry),
                );
            }
        }
    }
}

fn is_sensitive_dns_key(key: &str) -> bool {
    matches!(
        key,
        "access_key"
            | "access_key_id"
            | "access_key_secret"
            | "access_token"
            | "api_key"
            | "api_password"
            | "api_secret"
            | "api_token"
            | "application_key"
            | "application_secret"
            | "auth_token"
            | "client_secret"
            | "client_token"
            | "consumer_key"
            | "credentials"
            | "iam_token_b64"
            | "key"
            | "oauth_token"
            | "password"
            | "personal_access_token"
            | "private_key_password"
            | "private_key_pem"
            | "secret"
            | "secret_access_key"
            | "secret_key"
            | "security_token"
            | "session_token"
            | "token"
    )
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
