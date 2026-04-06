//! Configuration validator for `status`, `abort`, `block`, and `allow` directives.

use std::collections::HashSet;

use cidr::IpCidr;
use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::config::ServerConfigurationBlock;

/// Validator for http-response related directives.
#[derive(Default)]
pub struct HttpResponseValidator;

impl ConfigurationValidator for HttpResponseValidator {
    fn validate_block(
        &self,
        config: &ServerConfigurationBlock,
        used_directives: &mut HashSet<String>,
        _is_global: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Validate `abort` directive
        if let Some(entries) = config.directives.get("abort") {
            for entry in entries {
                if entry.args.len() > 1 {
                    return Err(
                        "'abort' directive requires zero or one argument (true or false)".into(),
                    );
                }
                if entry.args.len() > 0 && entry.args[0].as_boolean().is_none() {
                    return Err("'abort' value must be a boolean".into());
                }
            }
            used_directives.insert("abort".to_string());
        }

        // Validate `block` directives
        if let Some(entries) = config.directives.get("block") {
            for entry in entries {
                if entry.args.is_empty() {
                    return Err(
                        "'block' directive requires at least one IP or CIDR argument".into(),
                    );
                }
                for arg in &entry.args {
                    if let Some(s) = arg.as_str() {
                        if s.parse::<IpCidr>().is_err() {
                            return Err(
                                format!("Invalid IP or CIDR in 'block' directive: {s}").into()
                            );
                        }
                    } else {
                        return Err("'block' values must be strings (IP or CIDR)".into());
                    }
                }
            }
            used_directives.insert("block".to_string());
        }

        // Validate `allow` directives
        if let Some(entries) = config.directives.get("allow") {
            for entry in entries {
                if entry.args.is_empty() {
                    return Err(
                        "'allow' directive requires at least one IP or CIDR argument".into(),
                    );
                }
                for arg in &entry.args {
                    if let Some(s) = arg.as_str() {
                        if s.parse::<IpCidr>().is_err() {
                            return Err(
                                format!("Invalid IP or CIDR in 'allow' directive: {s}").into()
                            );
                        }
                    } else {
                        return Err("'allow' values must be strings (IP or CIDR)".into());
                    }
                }
            }
            used_directives.insert("allow".to_string());
        }

        // Validate `status` directives
        if let Some(entries) = config.directives.get("status") {
            for entry in entries {
                if entry.args.is_empty() {
                    return Err(
                        "'status' directive requires a status code as its first argument".into(),
                    );
                }

                // First arg must be the status code (integer)
                let status_code = entry.args[0]
                    .as_number()
                    .ok_or("'status' code must be an integer")?;

                if !(100..=599).contains(&status_code) {
                    return Err(format!(
                        "'status' code must be a valid HTTP status code (100-599), got {status_code}"
                    )
                    .into());
                }

                // Validate child block directives
                if let Some(children) = &entry.children {
                    for child_name in children.directives.keys() {
                        match child_name.as_str() {
                            "url" | "regex" | "location" | "body" => {
                                // Each should be a string value
                                if let Some(child_entries) = children.directives.get(child_name) {
                                    for child_entry in child_entries {
                                        if child_entry.args.is_empty() {
                                            return Err(format!(
                                                "'{child_name}' requires a string value"
                                            )
                                            .into());
                                        }
                                        if child_entry.args[0].as_str().is_none() {
                                            return Err(format!(
                                                "'{child_name}' value must be a string"
                                            )
                                            .into());
                                        }
                                    }
                                }
                            }
                            unknown => {
                                return Err(format!(
                                    "Unknown directive '{unknown}' inside 'status' block"
                                )
                                .into());
                            }
                        }
                    }

                    // Validate regex if present
                    if let Some(regex_entries) = children.directives.get("regex") {
                        for entry in regex_entries {
                            if let Some(regex_str) = entry.args.first().and_then(|v| v.as_str()) {
                                if fancy_regex::Regex::new(regex_str).is_err() {
                                    return Err(format!("Invalid regular expression in 'regex' directive: {regex_str}").into());
                                }
                            }
                        }
                    }
                }
            }
            used_directives.insert("status".to_string());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::config::{ServerConfigurationDirectiveEntry, ServerConfigurationValue};
    use std::collections::HashMap;
    use std::sync::Arc;

    fn make_value_number(n: i64) -> ServerConfigurationValue {
        ServerConfigurationValue::Number(n, None)
    }

    fn make_value_string(s: &str) -> ServerConfigurationValue {
        ServerConfigurationValue::String(s.to_string(), None)
    }

    fn make_value_bool(b: bool) -> ServerConfigurationValue {
        ServerConfigurationValue::Boolean(b, None)
    }

    fn make_block(
        directives: Vec<(&str, Vec<ServerConfigurationDirectiveEntry>)>,
    ) -> ServerConfigurationBlock {
        let mut d = HashMap::new();
        for (name, entries) in directives {
            d.insert(name.to_string(), entries);
        }
        ServerConfigurationBlock {
            directives: Arc::new(d),
            matchers: HashMap::new(),
            span: None,
        }
    }

    #[test]
    fn accepts_valid_abort() {
        let block = make_block(vec![(
            "abort",
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_bool(true)],
                children: None,
                span: None,
            }],
        )]);
        let mut used = HashSet::new();
        let validator = HttpResponseValidator;
        assert!(validator.validate_block(&block, &mut used, false).is_ok());
        assert!(used.contains("abort"));
    }

    #[test]
    fn rejects_non_boolean_abort() {
        let block = make_block(vec![(
            "abort",
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_string("yes")],
                children: None,
                span: None,
            }],
        )]);
        let mut used = HashSet::new();
        let validator = HttpResponseValidator;
        assert!(validator.validate_block(&block, &mut used, false).is_err());
    }

    #[test]
    fn accepts_valid_block_ips() {
        let block = make_block(vec![(
            "block",
            vec![ServerConfigurationDirectiveEntry {
                args: vec![
                    make_value_string("10.0.0.0/8"),
                    make_value_string("192.168.1.100"),
                ],
                children: None,
                span: None,
            }],
        )]);
        let mut used = HashSet::new();
        let validator = HttpResponseValidator;
        assert!(validator.validate_block(&block, &mut used, false).is_ok());
        assert!(used.contains("block"));
    }

    #[test]
    fn rejects_invalid_cidr_in_block() {
        let block = make_block(vec![(
            "block",
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_string("not-an-ip")],
                children: None,
                span: None,
            }],
        )]);
        let mut used = HashSet::new();
        let validator = HttpResponseValidator;
        assert!(validator.validate_block(&block, &mut used, false).is_err());
    }

    #[test]
    fn accepts_valid_status_code() {
        let block = make_block(vec![(
            "status",
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_number(404)],
                children: None,
                span: None,
            }],
        )]);
        let mut used = HashSet::new();
        let validator = HttpResponseValidator;
        assert!(validator.validate_block(&block, &mut used, false).is_ok());
        assert!(used.contains("status"));
    }

    #[test]
    fn rejects_out_of_range_status_code() {
        let block = make_block(vec![(
            "status",
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_number(999)],
                children: None,
                span: None,
            }],
        )]);
        let mut used = HashSet::new();
        let validator = HttpResponseValidator;
        assert!(validator.validate_block(&block, &mut used, false).is_err());
    }

    #[test]
    fn accepts_status_with_valid_child_directives() {
        let child_block = make_block(vec![
            (
                "url",
                vec![ServerConfigurationDirectiveEntry {
                    args: vec![make_value_string("/missing")],
                    children: None,
                    span: None,
                }],
            ),
            (
                "body",
                vec![ServerConfigurationDirectiveEntry {
                    args: vec![make_value_string("Not found")],
                    children: None,
                    span: None,
                }],
            ),
        ]);

        let block = make_block(vec![(
            "status",
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_number(404)],
                children: Some(child_block),
                span: None,
            }],
        )]);
        let mut used = HashSet::new();
        let validator = HttpResponseValidator;
        assert!(validator.validate_block(&block, &mut used, false).is_ok());
    }

    #[test]
    fn rejects_unknown_child_directive_in_status() {
        let child_block = make_block(vec![(
            "foo",
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_string("bar")],
                children: None,
                span: None,
            }],
        )]);

        let block = make_block(vec![(
            "status",
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_number(403)],
                children: Some(child_block),
                span: None,
            }],
        )]);
        let mut used = HashSet::new();
        let validator = HttpResponseValidator;
        assert!(validator.validate_block(&block, &mut used, false).is_err());
    }

    #[test]
    fn rejects_invalid_regex_in_status() {
        let child_block = make_block(vec![(
            "regex",
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_string("[invalid")],
                children: None,
                span: None,
            }],
        )]);

        let block = make_block(vec![(
            "status",
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_number(410)],
                children: Some(child_block),
                span: None,
            }],
        )]);
        let mut used = HashSet::new();
        let validator = HttpResponseValidator;
        assert!(validator.validate_block(&block, &mut used, false).is_err());
    }

    #[test]
    fn skips_block_without_directives() {
        let block = make_block(vec![]);
        let mut used = HashSet::new();
        let validator = HttpResponseValidator;
        assert!(validator.validate_block(&block, &mut used, false).is_ok());
    }
}
