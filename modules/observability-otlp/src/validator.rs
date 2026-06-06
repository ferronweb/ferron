use ferron_core::{
    config::{
        validator::{
            validate_scoped_block_flat, ConfigurationValidator,
            ConfigurationValidatorDiagnosticKind,
        },
        ServerConfigurationValue,
    },
    validate_directive, validate_nested,
};

pub struct OtlpObservabilityConfigurationValidator;

impl ConfigurationValidator for OtlpObservabilityConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        validator_ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        validate_scoped_block_flat(config, validator_ctx, "format", "logformat", Some("text"))?;

        validate_directive!(config, validator_ctx.used_directives, logs, optional args(1) => [ServerConfigurationValue::String(_, _)], {
            let mut sub = std::collections::HashSet::new();
            validate_nested!(logs, used(sub), protocol, optional args(1) => [ServerConfigurationValue::String(_, _)]);
            validate_nested!(logs, used(sub), authorization, optional args(1) => [ServerConfigurationValue::String(_, _)]);
            ferron_core::check_unused_subdirectives!(logs, sub, &mut validator_ctx.diagnostics, validator_ctx.scope.clone());
        });

        validate_directive!(config, validator_ctx.used_directives, metrics, optional args(1) => [ServerConfigurationValue::String(_, _)], {
            let mut sub = std::collections::HashSet::new();
            validate_nested!(metrics, used(sub), protocol, optional args(1) => [ServerConfigurationValue::String(_, _)]);
            validate_nested!(metrics, used(sub), authorization, optional args(1) => [ServerConfigurationValue::String(_, _)]);
            ferron_core::check_unused_subdirectives!(metrics, sub, &mut validator_ctx.diagnostics, validator_ctx.scope.clone());
        });

        validate_directive!(config, validator_ctx.used_directives, traces, optional args(1) => [ServerConfigurationValue::String(_, _)], {
            let mut sub = std::collections::HashSet::new();
            validate_nested!(traces, used(sub), protocol, optional args(1) => [ServerConfigurationValue::String(_, _)]);
            validate_nested!(traces, used(sub), authorization, optional args(1) => [ServerConfigurationValue::String(_, _)]);
            ferron_core::check_unused_subdirectives!(traces, sub, &mut validator_ctx.diagnostics, validator_ctx.scope.clone());
        });

        validate_directive!(config, validator_ctx.used_directives, service_name, optional args(1) => [ServerConfigurationValue::String(_, _)], {});

        validate_directive!(
            config,
            validator_ctx.used_directives,
            no_verification,
            optional,
            {}
        );

        validate_directive!(config, validator_ctx.used_directives, log_style, optional args(1) => [ServerConfigurationValue::String(_, _)], {
            // Check the value is recognized
            if let Some(value) = config.get_value("log_style").and_then(|v| v.as_str()) {
                if crate::config::parse_log_style(value).is_none() {
                    let err: Box<dyn std::error::Error> = format!(
                            "Invalid `log_style` value '{}': must be 'legacy' or 'modern'",
                            value
                        ).into();
                    Err(err)?
                }
            }
        });

        // When `log_style modern` is set, the `format` directive is ignored.
        // Emit a best-practice warning so the operator knows.
        let log_style_is_modern = config
            .get_value("log_style")
            .and_then(|v| v.as_str())
            .and_then(crate::config::parse_log_style)
            == Some(crate::config::LogStyle::Modern);
        if log_style_is_modern && config.directives.contains_key("format") {
            validator_ctx
                .diagnostics
                .push(validator_ctx.create_diagnostic(
                    ConfigurationValidatorDiagnosticKind::BestPracticeViolation,
                    "The `format` directive is ignored when `log_style` is `modern`".to_string(),
                    None,
                ));
        }

        // Validate `baggage { key "..." { ... } }` block
        if let Some(baggage_entries) = config.directives.get("baggage") {
            validator_ctx.used_directives.insert("baggage".to_string());
            for baggage_entry in baggage_entries {
                if let Some(children) = &baggage_entry.children {
                    validate_baggage_block(children, validator_ctx)?;
                }
            }
        }

        Ok(())
    }
}

/// Validate the contents of a `baggage { ... }` block.
fn validate_baggage_block(
    block: &ferron_core::config::ServerConfigurationBlock,
    validator_ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(key_entries) = block.directives.get("key") {
        for key_entry in key_entries {
            // Each `key` must have exactly 1 string argument (the baggage key name)
            if key_entry.args.len() != 1 {
                let err: Box<dyn std::error::Error> = format!(
                    "Invalid `baggage key` directive: \
                    expected 1 argument (the baggage key name), got {};",
                    key_entry.args.len()
                )
                .into();
                Err(err)?
            }
            if !matches!(&key_entry.args[0], ServerConfigurationValue::String(_, _)) {
                let err: Box<dyn std::error::Error> =
                    "Invalid `baggage key` directive: argument must be a string"
                        .to_string()
                        .into();
                Err(err)?;
            }

            // Validate children of the key block
            if let Some(children) = &key_entry.children {
                let mut sub = std::collections::HashSet::new();

                // `attribute` — optional string
                validate_nested!(children, used(sub), attribute, optional args(1) => [ServerConfigurationValue::String(_, _)]);

                // `signals` — optional, args are signal names
                if let Some(signal_entries) = children.directives.get("signals") {
                    sub.insert("signals".to_string());
                    for signal_entry in signal_entries {
                        for arg in &signal_entry.args {
                            if let Some(name) = arg.as_str() {
                                if name != "traces" && name != "logs" && name != "metrics" {
                                    let err: Box<dyn std::error::Error> = format!(
                                        "Invalid signal name '{}' in `baggage key` block: must be one of 'traces', 'logs', 'metrics'",
                                        name
                                    )
                                    .into();
                                    Err(err)?;
                                }
                            } else {
                                let err: Box<dyn std::error::Error> =
                                    "Invalid `signals` value: must be a string"
                                        .to_string()
                                        .into();
                                Err(err)?;
                            }
                        }
                    }
                }

                // `max_distinct` — optional number
                if let Some(max_entries) = children.directives.get("max_distinct") {
                    sub.insert("max_distinct".to_string());
                    for max_entry in max_entries {
                        if max_entry.args.len() != 1 {
                            let err: Box<dyn std::error::Error> =
                                "Invalid `max_distinct` directive: expected exactly 1 argument"
                                    .to_string()
                                    .into();
                            Err(err)?;
                        } else if !matches!(
                            &max_entry.args[0],
                            ServerConfigurationValue::Number(_, _)
                        ) {
                            let err: Box<dyn std::error::Error> =
                                "Invalid `max_distinct` value: must be a number"
                                    .to_string()
                                    .into();
                            Err(err)?;
                        }
                    }
                }

                ferron_core::check_unused_subdirectives!(
                    children,
                    sub,
                    &mut validator_ctx.diagnostics,
                    validator_ctx.scope.clone()
                );
            }
        }
    }

    Ok(())
}
