use std::net::SocketAddr;

use ferron_core::config::validator::{ConfigurationValidator, ConfigurationValidatorContext};
use ferron_core::config::{
    ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationSpan,
    ServerConfigurationValue,
};

pub struct AdminConfigurationValidator;

impl ConfigurationValidator for AdminConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let is_global = ctx.is_global;
        let used_directives = &mut ctx.used_directives;

        if is_global {
            ferron_core::validate_directive!(config, used_directives, admin, optional
                args(1) => [ServerConfigurationValue::Boolean(_, _)], {

                let mut sub = std::collections::HashSet::new();

                // Listen address
                ferron_core::validate_nested!(admin, used(sub), listen, args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)]);

                // Endpoint flags
                ferron_core::validate_nested!(admin, used(sub), health, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);
                ferron_core::validate_nested!(admin, used(sub), status, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);
                ferron_core::validate_nested!(admin, used(sub), config, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);
                ferron_core::validate_nested!(admin, used(sub), reload, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);
                ferron_core::validate_nested!(admin, used(sub), reload_get, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);
                ferron_core::validate_nested!(admin, used(sub), runtime, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);

                add_admin_best_practice_diagnostics(admin, ctx);
                ferron_core::check_unused_subdirectives!(admin, sub, &mut ctx.diagnostics, ctx.scope.clone());
            });
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

fn add_admin_best_practice_diagnostics(
    admin: &ServerConfigurationBlock,
    ctx: &mut ConfigurationValidatorContext,
) {
    let Some(listen_entry) = admin
        .directives
        .get("listen")
        .and_then(|entries| entries.first())
    else {
        return;
    };

    let Some(listen) = listen_entry
        .args
        .first()
        .and_then(|value| value.as_string_with_interpolations(&std::collections::HashMap::new()))
    else {
        return;
    };

    let Ok(addr) = listen.parse::<SocketAddr>() else {
        return;
    };

    if !addr.ip().is_loopback() {
        ctx.add_best_practice_violation(
            "`admin.listen` is not bound to a loopback address; the admin API is unauthenticated, unencrypted, and should only be reachable through a trusted local or protected management path",
            entry_span(listen_entry),
        );
    }
}
