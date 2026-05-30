use ferron_core::config::{
    validator::{ConfigurationValidator, ConfigurationValidatorContext},
    ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationSpan,
    ServerConfigurationValue,
};

pub struct DnsStalwartConfigurationValidator;

impl ConfigurationValidator for DnsStalwartConfigurationValidator {
    fn validate_block(
        &self,
        config: &ServerConfigurationBlock,
        ctx: &mut ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
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
) -> Result<(), Box<dyn std::error::Error>> {
    // Mark provider as used
    ctx.used_directives.insert("provider".to_string());

    match provider {
        // Simple single-credential providers
        "arvancloud" => {
            req_str(config, ctx, provider, "api_key")?;
        }
        "bunny" => {
            req_str(config, ctx, provider, "api_key")?;
        }
        "cloudflare" => {
            req_str(config, ctx, provider, "api_key")?;
        }
        "ddnss" => {
            req_str(config, ctx, provider, "key")?;
        }
        "desec" => {
            req_str(config, ctx, provider, "auth_token")?;
        }
        "digitalocean" => {
            req_str(config, ctx, provider, "auth_token")?;
        }
        "dreamhost" => {
            req_str(config, ctx, provider, "api_key")?;
        }
        "duckdns" => {
            req_str(config, ctx, provider, "token")?;
        }
        "dynu" => {
            req_str(config, ctx, provider, "api_key")?;
        }
        "freemyip" => {
            req_str(config, ctx, provider, "token")?;
        }
        "gandiv5" => {
            req_str(config, ctx, provider, "personal_access_token")?;
        }
        "gcore" => {
            req_str(config, ctx, provider, "api_token")?;
        }
        "hetzner" => {
            req_str(config, ctx, provider, "api_token")?;
        }
        "hostingde" => {
            req_str(config, ctx, provider, "api_key")?;
        }
        "hostinger" => {
            req_str(config, ctx, provider, "api_token")?;
        }
        "infomaniak" => {
            req_str(config, ctx, provider, "api_token")?;
        }
        "ionos" => {
            req_str(config, ctx, provider, "api_key")?;
        }
        "ipv64" => {
            req_str(config, ctx, provider, "api_key")?;
        }
        "linode" => {
            req_str(config, ctx, provider, "api_token")?;
        }
        "namesilo" => {
            req_str(config, ctx, provider, "api_token")?;
        }
        "netlify" => {
            req_str(config, ctx, provider, "access_token")?;
        }
        "ns1" => {
            req_str(config, ctx, provider, "api_key")?;
        }
        "safedns" => {
            req_str(config, ctx, provider, "auth_token")?;
        }
        "scaleway" => {
            req_str(config, ctx, provider, "api_token")?;
        }
        "vultr" => {
            req_str(config, ctx, provider, "api_key")?;
        }

        // Dual-credential providers
        "baiducloud" => {
            req_str(config, ctx, provider, "access_key_id")?;
            req_str(config, ctx, provider, "access_key_secret")?;
        }
        "constellix" => {
            req_str(config, ctx, provider, "api_key")?;
            req_str(config, ctx, provider, "secret_key")?;
        }
        "dnsimple" => {
            req_str(config, ctx, provider, "oauth_token")?;
            req_str(config, ctx, provider, "account_id")?;
        }
        "dnsmadeeasy" => {
            req_str(config, ctx, provider, "api_key")?;
            req_str(config, ctx, provider, "api_secret")?;
        }
        "domeneshop" => {
            req_str(config, ctx, provider, "api_token")?;
            req_str(config, ctx, provider, "api_secret")?;
        }
        "easydns" => {
            req_str(config, ctx, provider, "token")?;
            req_str(config, ctx, provider, "key")?;
        }
        "exoscale" => {
            req_str(config, ctx, provider, "api_key")?;
            req_str(config, ctx, provider, "api_secret")?;
        }
        "glesys" => {
            req_str(config, ctx, provider, "api_user")?;
            req_str(config, ctx, provider, "api_key")?;
        }
        "godaddy" => {
            req_str(config, ctx, provider, "api_key")?;
            req_str(config, ctx, provider, "api_secret")?;
        }
        "ibmcloud" => {
            req_str(config, ctx, provider, "username")?;
            req_str(config, ctx, provider, "api_key")?;
        }
        "luadns" => {
            req_str(config, ctx, provider, "api_username")?;
            req_str(config, ctx, provider, "api_token")?;
        }
        "mythicbeasts" => {
            req_str(config, ctx, provider, "username")?;
            req_str(config, ctx, provider, "password")?;
        }
        "namedotcom" => {
            req_str(config, ctx, provider, "username")?;
            req_str(config, ctx, provider, "api_token")?;
        }
        "nifcloud" => {
            req_str(config, ctx, provider, "api_key")?;
            req_str(config, ctx, provider, "api_secret")?;
        }

        // Triple-credential providers
        "cpanel" => {
            req_str(config, ctx, provider, "base_url")?;
            req_str(config, ctx, provider, "username")?;
            req_str(config, ctx, provider, "token")?;
        }
        "netcup" => {
            req_str(config, ctx, provider, "customer_number")?;
            req_str(config, ctx, provider, "api_key")?;
            req_str(config, ctx, provider, "api_password")?;
        }
        "plesk" => {
            req_str(config, ctx, provider, "base_url")?;
            req_str(config, ctx, provider, "api_key")?;
        }
        "porkbun" => {
            req_str(config, ctx, provider, "api_key")?;
            req_str(config, ctx, provider, "api_secret")?;
        }
        "spaceship" => {
            req_str(config, ctx, provider, "api_key")?;
            req_str(config, ctx, provider, "api_secret")?;
        }
        "websupport" => {
            req_str(config, ctx, provider, "api_key")?;
            req_str(config, ctx, provider, "secret")?;
        }

        // Providers with optional directives
        "alidns" => {
            req_str(config, ctx, provider, "access_key_id")?;
            req_str(config, ctx, provider, "access_key_secret")?;
            opt_str(ctx, "region");
            opt_str(ctx, "security_token");
            opt_str(ctx, "line");
        }
        "autodns" => {
            req_str(config, ctx, provider, "username")?;
            req_str(config, ctx, provider, "password")?;
            opt_num(ctx, "context");
        }
        "azuredns" => {
            req_str(config, ctx, provider, "tenant_id")?;
            req_str(config, ctx, provider, "client_id")?;
            req_str(config, ctx, provider, "client_secret")?;
            req_str(config, ctx, provider, "subscription_id")?;
            req_str(config, ctx, provider, "resource_group")?;
            req_enum(
                config,
                ctx,
                provider,
                "endpoint",
                &["AzurePublicCloud", "AzureChinaCloud", "AzureUSGovernment"],
            )?;
        }
        "bluecatv2" => {
            req_str(config, ctx, provider, "server_url")?;
            req_str(config, ctx, provider, "username")?;
            req_str(config, ctx, provider, "password")?;
            req_str(config, ctx, provider, "config_name")?;
            req_str(config, ctx, provider, "view_name")?;
            opt_bool(ctx, "skip_deploy");
        }
        "cloudns" => {
            req_str(config, ctx, provider, "password")?;
            opt_str(ctx, "auth_id");
            opt_str(ctx, "sub_auth_id");
        }
        "edgedns" => {
            req_str(config, ctx, provider, "host")?;
            req_str(config, ctx, provider, "client_token")?;
            req_str(config, ctx, provider, "client_secret")?;
            req_str(config, ctx, provider, "access_token")?;
            opt_str(ctx, "account_switch_key");
        }
        "googlecloud" => {
            req_str(config, ctx, provider, "service_account_json")?;
            req_str(config, ctx, provider, "project_id")?;
            opt_str(ctx, "managed_zone");
            opt_bool(ctx, "private_zone");
            opt_str(ctx, "impersonate_service_account");
        }
        "huaweicloud" => {
            req_str(config, ctx, provider, "access_key_id")?;
            req_str(config, ctx, provider, "access_key_secret")?;
            req_str(config, ctx, provider, "region")?;
        }
        "infoblox" => {
            req_str(config, ctx, provider, "host")?;
            req_str(config, ctx, provider, "username")?;
            req_str(config, ctx, provider, "password")?;
            opt_str(ctx, "port");
            opt_str(ctx, "wapi_version");
            opt_str(ctx, "dns_view");
        }
        "inwx" => {
            req_str(config, ctx, provider, "username")?;
            req_str(config, ctx, provider, "password")?;
            opt_str(ctx, "shared_secret");
            opt_bool(ctx, "sandbox");
        }
        "lightsail" => {
            req_str(config, ctx, provider, "access_key_id")?;
            req_str(config, ctx, provider, "secret_access_key")?;
            opt_str(ctx, "region");
            opt_str(ctx, "session_token");
            opt_str(ctx, "domain");
        }
        "namecheap" => {
            req_str(config, ctx, provider, "api_key")?;
            req_str(config, ctx, provider, "api_secret")?;
            req_str(config, ctx, provider, "client_ip")?;
            opt_str(ctx, "username");
        }
        "oraclecloud" => {
            req_str(config, ctx, provider, "tenancy_ocid")?;
            req_str(config, ctx, provider, "user_ocid")?;
            req_str(config, ctx, provider, "fingerprint")?;
            req_str(config, ctx, provider, "private_key_pem")?;
            req_str(config, ctx, provider, "region")?;
            req_str(config, ctx, provider, "compartment_ocid")?;
            opt_str(ctx, "private_key_password");
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
        }
        "tencentcloud" => {
            req_str(config, ctx, provider, "secret_id")?;
            req_str(config, ctx, provider, "secret_key")?;
            opt_str(ctx, "region");
            opt_str(ctx, "session_token");
        }
        "ultradns" => {
            req_str(config, ctx, provider, "username")?;
            req_str(config, ctx, provider, "password")?;
            opt_str(ctx, "endpoint");
        }
        "vercel" => {
            req_str(config, ctx, provider, "auth_token")?;
            opt_str(ctx, "team_id");
        }
        "volcengine" => {
            req_str(config, ctx, provider, "access_key")?;
            req_str(config, ctx, provider, "secret_key")?;
            opt_str(ctx, "region");
            opt_str(ctx, "host");
            opt_str(ctx, "scheme");
        }
        "yandexcloud" => {
            req_str(config, ctx, provider, "iam_token_b64")?;
            req_str(config, ctx, provider, "folder_id")?;
        }

        // Special cases
        "hurricane" => {
            req_str(config, ctx, provider, "credentials")?;
        }
        "joker" => {
            // Conditional: (username AND password) OR (api_key)
            opt_str(ctx, "api_key");
            opt_str(ctx, "username");
            opt_str(ctx, "password");
            let has_api_key = config.get_value("api_key").is_some();
            let has_username = config.get_value("username").is_some();
            let has_password = config.get_value("password").is_some();
            if !(has_api_key || has_username && has_password) {
                return Err(format!(
                    "No API key or username/password provided for '{provider}' DNS provider"
                )
                .into());
            }
        }
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
) -> Result<(), Box<dyn std::error::Error>> {
    ctx.used_directives.insert(key.to_string());
    match config.get_value(key) {
        Some(ServerConfigurationValue::String(_, _))
        | Some(ServerConfigurationValue::InterpolatedString(_, _)) => Ok(()),
        _ => Err(
            format!("Missing or invalid directive '{key}' for '{provider}' DNS provider").into(),
        ),
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
fn opt_num(ctx: &mut ConfigurationValidatorContext, key: &str) {
    ctx.used_directives.insert(key.to_string());
}

/// Validate that a required directive is present and its string value is one of the allowed values.
fn req_enum(
    config: &ServerConfigurationBlock,
    ctx: &mut ConfigurationValidatorContext,
    provider: &str,
    key: &str,
    allowed: &[&str],
) -> Result<(), Box<dyn std::error::Error>> {
    ctx.used_directives.insert(key.to_string());
    let value = match config.get_value(key) {
        Some(ServerConfigurationValue::String(s, _)) => s.as_str(),
        Some(ServerConfigurationValue::InterpolatedString(_, _)) => {
            // Interpolated strings are validated at runtime
            return Ok(());
        }
        _ => {
            return Err(format!(
                "Missing or invalid directive '{key}' for '{provider}' DNS provider"
            )
            .into());
        }
    };
    if !allowed.contains(&value) {
        return Err(format!(
            "Invalid value '{value}' for directive '{key}' in '{provider}' DNS provider: must be one of {}",
            allowed
                .iter()
                .map(|v| format!("'{v}'"))
                .collect::<Vec<_>>()
                .join(", ")
        )
        .into());
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
