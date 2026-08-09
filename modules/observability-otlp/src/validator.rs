use ferron_core::config::validator::{
    entry_span, validate_scoped_block_flat, ConfigurationValidationError, ConfigurationValidator,
};
use ferron_core::config::ServerConfigurationValue;
use ferron_core::{validate_directive, validate_nested};

pub struct OtlpObservabilityConfigurationValidator;

impl ConfigurationValidator for OtlpObservabilityConfigurationValidator {
    #[inline]
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        validator_ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        validate_scoped_block_flat(config, validator_ctx, "format", "logformat", Some("text"))?;

        validate_directive!(config, validator_ctx.used_directives, logs, optional args(1) => [ServerConfigurationValue::String(_, _)], {
            let mut sub = std::collections::HashSet::new();
            validate_nested!(logs, used(sub), protocol, optional args(1) => [ServerConfigurationValue::String(_, _)]);
            validate_nested!(logs, used(sub), authorization, optional args(1) => [ServerConfigurationValue::String(_, _)]);
            validate_nested!(logs, used(sub), export_interval, optional args(1) => [ServerConfigurationValue::Number(_, _) | ServerConfigurationValue::Float(_, _) | ServerConfigurationValue::String(_, _)]);
            validate_nested!(logs, used(sub), export_batch_size, optional args(1) => [ServerConfigurationValue::Number(_, _)]);
            if let Some(entries) = logs.directives.get("export_batch_size") {
                for entry in entries {
                    if entry.get_value().and_then(|v| v.as_number()).is_some_and(|n| n < 1) {
                        return Err(ConfigurationValidationError::from("Export batch size for OTLP logs cannot be smaller than 1".to_string())
                        .with_span(
                            entry.span.clone()
                        ));
                    }
                }
            }
            validate_nested!(logs, used(sub), gzip, optional args(?) => [ServerConfigurationValue::Boolean(_, _)]);
            ferron_core::check_unused_subdirectives!(logs, sub, &mut validator_ctx.diagnostics, validator_ctx.scope.clone());
        });

        validate_directive!(config, validator_ctx.used_directives, metrics, optional args(1) => [ServerConfigurationValue::String(_, _)], {
            let mut sub = std::collections::HashSet::new();
            validate_nested!(metrics, used(sub), protocol, optional args(1) => [ServerConfigurationValue::String(_, _)]);
            validate_nested!(metrics, used(sub), authorization, optional args(1) => [ServerConfigurationValue::String(_, _)]);
            validate_nested!(metrics, used(sub), read_interval, optional args(1) => [ServerConfigurationValue::Number(_, _) | ServerConfigurationValue::Float(_, _) | ServerConfigurationValue::String(_, _)]);
            validate_nested!(metrics, used(sub), gzip, optional args(?) => [ServerConfigurationValue::Boolean(_, _)]);
            validate_nested!(metrics, used(sub), exemplars, optional args(?) => [ServerConfigurationValue::Boolean(_, _)]);
            validate_nested!(metrics, used(sub), native_histograms, optional args(?) => [ServerConfigurationValue::Boolean(_, _)]);
            ferron_core::check_unused_subdirectives!(metrics, sub, &mut validator_ctx.diagnostics, validator_ctx.scope.clone());
        });

        validate_directive!(config, validator_ctx.used_directives, traces, optional args(1) => [ServerConfigurationValue::String(_, _)], {
            let mut sub = std::collections::HashSet::new();
            validate_nested!(traces, used(sub), protocol, optional args(1) => [ServerConfigurationValue::String(_, _)]);
            validate_nested!(traces, used(sub), authorization, optional args(1) => [ServerConfigurationValue::String(_, _)]);
            validate_nested!(traces, used(sub), export_interval, optional args(1) => [ServerConfigurationValue::Number(_, _) | ServerConfigurationValue::Float(_, _) | ServerConfigurationValue::String(_, _)]);
            validate_nested!(traces, used(sub), export_batch_size, optional args(1) => [ServerConfigurationValue::Number(_, _)]);
            if let Some(entries) = traces.directives.get("export_batch_size") {
                for entry in entries {
                    if entry.get_value().and_then(|v| v.as_number()).is_some_and(|n| n < 1) {
                        return Err(ConfigurationValidationError::from("Export batch size for OTLP logs cannot be smaller than 1".to_string())
                        .with_span(
                            entry.span.clone()
                        ));
                    }
                }
            }
            validate_nested!(traces, used(sub), gzip, optional args(?) => [ServerConfigurationValue::Boolean(_, _)]);
            ferron_core::check_unused_subdirectives!(traces, sub, &mut validator_ctx.diagnostics, validator_ctx.scope.clone());
        });

        validate_directive!(config, validator_ctx.used_directives, service_name, optional args(1) => [ServerConfigurationValue::String(_, _)], {});
        if !validator_ctx.used_directives.contains("service_name") {
            validator_ctx.add_best_practice_violation(
                "OTLP configured without an explicit `service_name`; \
                data might be attributed incorrectly for multi-service environments",
                config.span.clone(),
            );
        }

        validate_directive!(
            config,
            validator_ctx.used_directives,
            no_verification,
            optional,
            {}
        );
        if config.get_flag("no_verification") {
            let span = config
                .directives
                .get("no_verification")
                .and_then(|s| s.first())
                .and_then(|s| s.span.clone());
            validator_ctx.add_best_practice_violation(
                "`no_verification` disables TLS certificate verification for OTLP endpoints; \
                use it only for testing or tightly controlled internal OpenTelemetry services",
                span,
            );
        }

        validate_directive!(config, validator_ctx.used_directives, log_style, optional args(1) => [ServerConfigurationValue::String(_, _)], {
            if let Some(value) = config.get_value("log_style").and_then(|v| v.as_str()) {
                if crate::config::parse_log_style(value).is_none() {
                    let err: ferron_core::config::validator::ConfigurationValidationError = format!(
                            "Invalid `log_style` value '{}': must be 'legacy' or 'modern'",
                            value
                        ).into();
                    Err(err)?
                }
            }
        });

        let log_style = config
            .get_value("log_style")
            .and_then(|v| v.as_str())
            .and_then(crate::config::parse_log_style)
            .unwrap_or_default();

        if log_style == crate::config::LogStyle::Modern && config.directives.contains_key("format")
        {
            // When `log_style modern` is set, the `format` directive is ignored.
            // Error out, so the operator knows.
            let err: ferron_core::config::validator::ConfigurationValidationError =
                "The `format` directive would be ignored when `log_style` is `modern`".into();
            Err(err)?;
        } else if log_style == crate::config::LogStyle::Legacy {
            // Emit a best-practice violation warning about `log_style legacy`
            let log_style_span = config
                .directives
                .get("log_style")
                .and_then(|s| s.first())
                .and_then(|s| s.span.clone());
            validator_ctx.add_best_practice_violation(
                "`log_style legacy` detected - OpenTelemetry logs may be harder to filter or aggregate",
                log_style_span
            );
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
#[inline]
fn validate_baggage_block(
    block: &ferron_core::config::ServerConfigurationBlock,
    validator_ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
    if let Some(key_entries) = block.directives.get("key") {
        for key_entry in key_entries {
            // Each `key` must have exactly 1 string argument (the baggage key name)
            if key_entry.args.len() != 1 {
                let err: ferron_core::config::validator::ConfigurationValidationError = format!(
                    "Invalid `baggage key` directive: \
                    expected 1 argument (the baggage key name), got {};",
                    key_entry.args.len()
                )
                .into();
                Err(err.with_span(entry_span(key_entry)))?
            }
            if !matches!(&key_entry.args[0], ServerConfigurationValue::String(_, _)) {
                let err: ferron_core::config::validator::ConfigurationValidationError =
                    "Invalid `baggage key` directive: argument must be a string"
                        .to_string()
                        .into();
                Err(err.with_span(entry_span(key_entry)))?;
            }

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
                                    let err: ferron_core::config::validator::ConfigurationValidationError = format!(
                                        "Invalid signal name '{}' in `baggage key` block: must be one of 'traces', 'logs', 'metrics'",
                                        name
                                    )
                                    .into();
                                    Err(err.with_span(entry_span(signal_entry)))?;
                                }
                            } else {
                                let err: ferron_core::config::validator::ConfigurationValidationError =
                                    "Invalid `signals` value: must be a string"
                                        .to_string()
                                        .into();
                                Err(err.with_span(entry_span(signal_entry)))?;
                            }
                        }
                    }
                }

                // `max_distinct` — optional number
                if let Some(max_entries) = children.directives.get("max_distinct") {
                    sub.insert("max_distinct".to_string());
                    for max_entry in max_entries {
                        if max_entry.args.len() != 1 {
                            let err: ferron_core::config::validator::ConfigurationValidationError =
                                "Invalid `max_distinct` directive: expected exactly 1 argument"
                                    .to_string()
                                    .into();
                            Err(err.with_span(entry_span(max_entry)))?;
                        } else if !matches!(
                            &max_entry.args[0],
                            ServerConfigurationValue::Number(_, _)
                                | ServerConfigurationValue::Boolean(false, _)
                        ) {
                            let err: ferron_core::config::validator::ConfigurationValidationError =
                                "Invalid `max_distinct` value: must be a number or `false`"
                                    .to_string()
                                    .into();
                            Err(err.with_span(entry_span(max_entry)))?;
                        }
                        if max_entry.args[0].as_boolean().is_some_and(|v| !v) {
                            validator_ctx.add_best_practice_violation(
                                "`max_distinct` set to `false`, high cardinality might be allowed.",
                                max_entry.span.clone(),
                            );
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
