use ferron_core::{
    config::{
        validator::{validate_scoped_block_flat, ConfigurationValidator},
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

            // Validate `sampling` sub-directive
            if let Some(sampling_entries) = config.directives.get("traces") {
                if let Some(traces_entry) = sampling_entries.first() {
                    if let Some(children) = &traces_entry.children {
                        if let Some(sampling_dirs) = children.directives.get("sampling") {
                            sub.insert("sampling".to_string());
                            for sampling_entry in sampling_dirs {
                                validate_sampling_directive(sampling_entry, validator_ctx)?;
                            }
                        }
                    }
                }
            }

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
        // Error out, so the operator knows.
        let log_style_is_modern = config
            .get_value("log_style")
            .and_then(|v| v.as_str())
            .and_then(crate::config::parse_log_style)
            == Some(crate::config::LogStyle::Modern);
        if log_style_is_modern && config.directives.contains_key("format") {
            let err: Box<dyn std::error::Error> =
                "The `format` directive would be ignored when `log_style` is `modern`".into();
            Err(err)?;
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
                } else {
                    // `max_distinct` not explicitly set, emit best practice violation warning
                    validator_ctx.add_best_practice_violation(
                        "`max_distinct` not explicitly set, high cardinality might be allowed.",
                        children.span.clone(),
                    );
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

/// Validate a `sampling` directive inside a `traces` block.
fn validate_sampling_directive(
    entry: &ferron_core::config::ServerConfigurationDirectiveEntry,
    _validator_ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
) -> Result<(), Box<dyn std::error::Error>> {
    // First arg must be a recognized sampling mode
    let mode = entry.args.first().and_then(|v| v.as_str());
    match mode {
        Some("always_on" | "always_off" | "parentbased_always_on") => {
            // No sub-directives expected
        }
        Some("traceidratio" | "parentbased_traceidratio") => {
            // Validate optional `ratio` sub-directive
            if let Some(children) = &entry.children {
                if let Some(ratio_entries) = children.directives.get("ratio") {
                    for ratio_entry in ratio_entries {
                        if ratio_entry.args.len() != 1 {
                            let err: Box<dyn std::error::Error> =
                                "Invalid `ratio` directive: expected exactly 1 argument (a float between 0.0 and 1.0)"
                                    .to_string()
                                    .into();
                            Err(err)?;
                        } else if !matches!(
                            &ratio_entry.args[0],
                            ServerConfigurationValue::Float(_, _)
                        ) {
                            let err: Box<dyn std::error::Error> =
                                "Invalid `ratio` value: must be a float between 0.0 and 1.0"
                                    .to_string()
                                    .into();
                            Err(err)?;
                        } else if let ServerConfigurationValue::Float(r, _) = &ratio_entry.args[0] {
                            if *r < 0.0 || *r > 1.0 {
                                let err: Box<dyn std::error::Error> =
                                    "Invalid `ratio` value: must be between 0.0 and 1.0"
                                        .to_string()
                                        .into();
                                Err(err)?;
                            }
                        }
                    }
                }
            }
        }
        Some("attribute_based") => {
            // Validate `rules { rule ... }` block
            if let Some(children) = &entry.children {
                if let Some(rules_entries) = children.directives.get("rules") {
                    for rules_entry in rules_entries {
                        if let Some(rules_block) = &rules_entry.children {
                            if let Some(rule_entries) = rules_block.directives.get("rule") {
                                for rule_entry in rule_entries {
                                    // Must have 2 or 3 args: match_type, attribute, [value]
                                    if rule_entry.args.len() < 2 || rule_entry.args.len() > 3 {
                                        let err: Box<dyn std::error::Error> = format!(
                                            "Invalid `rule` directive: expected 2 or 3 arguments (match_type, attribute, [value]), got {}",
                                            rule_entry.args.len()
                                        )
                                        .into();
                                        Err(err)?;
                                    }
                                    // First arg must be a recognized match type
                                    if let Some(match_type) = rule_entry.args.first().and_then(|v| v.as_str()) {
                                        match match_type {
                                            "exact" | "prefix" => {
                                                // Must have 3 args (match_type, attribute, value)
                                                if rule_entry.args.len() != 3 {
                                                    let err: Box<dyn std::error::Error> = format!(
                                                        "Invalid `{}` rule: expected 3 arguments (match_type, attribute, value), got {}",
                                                        match_type,
                                                        rule_entry.args.len()
                                                    )
                                                    .into();
                                                    Err(err)?;
                                                }
                                            }
                                            "exists" => {
                                                // Must have 2 args (match_type, attribute)
                                                if rule_entry.args.len() != 2 {
                                                    let err: Box<dyn std::error::Error> = format!(
                                                        "Invalid `exists` rule: expected 2 arguments (match_type, attribute), got {}",
                                                        rule_entry.args.len()
                                                    )
                                                    .into();
                                                    Err(err)?;
                                                }
                                            }
                                            other => {
                                                let err: Box<dyn std::error::Error> = format!(
                                                    "Invalid rule match type '{}': must be 'exact', 'prefix', or 'exists'",
                                                    other
                                                )
                                                .into();
                                                Err(err)?;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        Some(other) => {
            let err: Box<dyn std::error::Error> = format!(
                "Invalid sampling mode '{}': must be one of 'always_on', 'always_off', 'parentbased_always_on', 'traceidratio', 'parentbased_traceidratio', 'attribute_based'",
                other
            )
            .into();
            Err(err)?;
        }
        None => {
            let err: Box<dyn std::error::Error> =
                "The `sampling` directive requires a mode argument".to_string().into();
            Err(err)?;
        }
    }

    Ok(())
}
