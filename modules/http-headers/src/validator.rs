//! Configuration validation for the HTTP headers module.

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::str::FromStr;

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::config::ServerConfigurationBlock;
use http::header::HeaderName;

/// Configuration validator for the HTTP headers module.
pub struct HttpHeadersConfigurationValidator;

impl ConfigurationValidator for HttpHeadersConfigurationValidator {
    fn validate_block(
        &self,
        config: &ServerConfigurationBlock,
        used_directives: &mut HashSet<String>,
        _is_global: bool,
    ) -> Result<(), Box<dyn Error>> {
        // Validate header directives
        if let Some(entries) = config.directives.get("header") {
            used_directives.insert("header".to_string());
            for e in entries {
                if e.args.is_empty() {
                    return Err("Invalid `header` — requires at least one argument".into());
                }
                let first = e.args[0]
                    .as_str()
                    .ok_or("Invalid `header` — name must be a string")?;
                let (name, needs_value) = match first.chars().next() {
                    Some('+') => (&first[1..], true),
                    Some('-') => (&first[1..], false),
                    _ => (first, true),
                };
                HeaderName::from_str(name)
                    .map_err(|e| format!("Invalid `header` — invalid header name '{name}': {e}"))?;
                if needs_value
                    && e.args
                        .get(1)
                        .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
                        .is_none()
                {
                    return Err(
                        "Invalid `header` — requires a value for add/replace operations".into(),
                    );
                }
            }
        }

        // Validate cors directives
        if let Some(entries) = config.directives.get("cors") {
            used_directives.insert("cors".to_string());
            for e in entries {
                if let Some(block) = &e.children {
                    validate_cors_block(block, used_directives)?;
                }
            }
        }

        Ok(())
    }
}

fn validate_cors_block(
    block: &ServerConfigurationBlock,
    used: &mut HashSet<String>,
) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get("origins") {
        used.insert("origins".to_string());
        for e in entries {
            for arg in &e.args {
                if arg.as_str().is_none() {
                    return Err("Invalid `origins` — expected a string".into());
                }
            }
        }
    }
    if let Some(entries) = block.directives.get("methods") {
        used.insert("methods".to_string());
        for e in entries {
            for arg in &e.args {
                if arg.as_str().is_none() {
                    return Err("Invalid `methods` — expected a string".into());
                }
            }
        }
    }
    if let Some(entries) = block.directives.get("headers") {
        used.insert("headers".to_string());
        for e in entries {
            for arg in &e.args {
                if arg.as_str().is_none() {
                    return Err("Invalid `headers` — expected a string".into());
                }
            }
        }
    }
    if let Some(entries) = block.directives.get("credentials") {
        used.insert("credentials".to_string());
        for e in entries {
            if e.args.first().and_then(|v| v.as_boolean()).is_none() {
                return Err("Invalid `credentials` — expected a boolean".into());
            }
        }
    }
    if let Some(entries) = block.directives.get("max_age") {
        used.insert("max_age".to_string());
        for e in entries {
            if let Some(val) = e.args.first().and_then(|v| v.as_number()) {
                if val < 0 {
                    return Err("Invalid `max_age` — must be non-negative".into());
                }
            } else {
                return Err("Invalid `max_age` — expected a number".into());
            }
        }
    }
    if let Some(entries) = block.directives.get("expose_headers") {
        used.insert("expose_headers".to_string());
        for e in entries {
            for arg in &e.args {
                if arg.as_str().is_none() {
                    return Err("Invalid `expose_headers` — expected a string".into());
                }
            }
        }
    }
    Ok(())
}
