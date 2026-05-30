use ferron_core::{
    check_unused_subdirectives,
    config::{
        validator::ConfigurationValidator,
        ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationSpan,
        ServerConfigurationValue,
    },
    validate_directive, validate_nested,
};

pub struct ForwardedAuthenticationConfigurationValidator;

impl ConfigurationValidator for ForwardedAuthenticationConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let is_global = ctx.is_global;
        let mut concurrent_conns_warn: Option<ServerConfigurationSpan> = None;
        let mut no_verification_warn: Option<ServerConfigurationSpan> = None;
        {
            let used_directives = &mut ctx.used_directives;
            if is_global {
                // Manual validation for auth_to_concurrent_conns directive
                if let Some(directives) =
                    config.directives.get(stringify!(auth_to_concurrent_conns))
                {
                    used_directives
                        .insert(stringify!(auth_to_concurrent_conns).to_string());
                    for directive in directives {
                        if directive.args.len() != 1 {
                            return Err(format!(
                                "Invalid directive '{}': expected {} argument(s), got {}",
                                stringify!(auth_to_concurrent_conns),
                                1,
                                directive.args.len()
                            )
                            .into());
                        }
                        if !matches!(
                            directive.args[0],
                            ServerConfigurationValue::Number(n, _) if n > 0
                        ) && !matches!(
                            directive.args[0],
                            ServerConfigurationValue::Boolean(false, _)
                        ) {
                            return Err(format!(
                                "Invalid directive '{}': invalid type",
                                stringify!(auth_to_concurrent_conns)
                            )
                            .into());
                        }
                        if matches!(
                            directive.args[0],
                            ServerConfigurationValue::Boolean(false, _)
                        ) {
                            concurrent_conns_warn = entry_span(directive);
                        }
                    }
                };
            }

            validate_directive!(config, used_directives, auth_to, optional args(1) => [ServerConfigurationValue::Boolean(_, _) | ServerConfigurationValue::InterpolatedString(_, _) | ServerConfigurationValue::String(_, _)], {
                let mut sub = std::collections::HashSet::new();
                validate_nested!(auth_to, used(sub), url, args(1) => [ServerConfigurationValue::InterpolatedString(_, _) | ServerConfigurationValue::String(_, _)]);
                validate_nested!(auth_to, used(sub), unix, args(1) => [ServerConfigurationValue::InterpolatedString(_, _) | ServerConfigurationValue::String(_, _)]);
                validate_nested!(auth_to, used(sub), limit, args(1) => [ServerConfigurationValue::Number(_, _) | ServerConfigurationValue::Boolean(false, _)]);
                validate_nested!(auth_to, used(sub), idle_timeout, args(1) => [ServerConfigurationValue::Number(_, _) | ServerConfigurationValue::String(_, _) | ServerConfigurationValue::Boolean(false, _)]);
                validate_nested!(auth_to, used(sub), no_verification, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);
                if block_flag(auth_to, "no_verification") == Some(true) {
                    no_verification_warn = first_entry_span(auth_to, "no_verification");
                }
                validate_nested!(auth_to, used(sub), copy, args(*) => [ServerConfigurationValue::String(_, _)]);
                check_unused_subdirectives!(auth_to, sub, &mut ctx.diagnostics, ctx.scope.clone());
            });
        }

        if let Some(span) = concurrent_conns_warn {
            ctx.add_best_practice_violation(
                "`auth_to_concurrent_conns false` disables the global forwarded-auth connection limit; keep a bounded limit to protect authentication backends under load",
                Some(span),
            );
        }
        if let Some(span) = no_verification_warn {
            ctx.add_best_practice_violation(
                "`auth_to.no_verification` disables TLS certificate verification for the authentication backend; keep verification enabled outside tightly controlled internal test environments",
                Some(span),
            );
        }

        Ok(())
    }
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

fn first_entry_span(
    block: &ServerConfigurationBlock,
    directive: &str,
) -> Option<ServerConfigurationSpan> {
    block
        .directives
        .get(directive)
        .and_then(|entries| entries.first())
        .and_then(entry_span)
}

fn block_flag(block: &ServerConfigurationBlock, directive: &str) -> Option<bool> {
    block
        .directives
        .get(directive)
        .and_then(|entries| entries.first())
        .map(ServerConfigurationDirectiveEntry::get_flag)
}
