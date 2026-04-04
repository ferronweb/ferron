use std::collections::HashMap;
use std::sync::Arc;

use ferron_core::config::{
    ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue,
};

/// Transform alias directives (log, error_log, console_log) into an observability block.
/// Returns None if the alias is disabled (e.g., `log false`).
pub fn transform_observability_alias(
    directive_name: &str,
    directive: &ServerConfigurationDirectiveEntry,
) -> Result<Option<ServerConfigurationBlock>, Box<dyn std::error::Error>> {
    // Check if the directive is disabled
    if directive
        .args
        .first()
        .and_then(|a| a.as_boolean())
        .map(|b| !b)
        .unwrap_or(false)
    {
        return Ok(None);
    }

    let mut directives = HashMap::new();

    match directive_name {
        "log" => {
            // log /path/to/access.log { ... } -> observability { provider file; access_log /path/to/access.log; ... }
            directives.insert(
                "provider".to_string(),
                vec![ServerConfigurationDirectiveEntry {
                    args: vec![ServerConfigurationValue::String("file".to_string(), None)],
                    children: None,
                    span: directive.span.clone(),
                }],
            );

            // If first arg is a string, it's the access_log path
            if let Some(path_value) = directive.args.first().and_then(|v| v.as_str()) {
                directives.insert(
                    "access_log".to_string(),
                    vec![ServerConfigurationDirectiveEntry {
                        args: vec![ServerConfigurationValue::String(
                            path_value.to_string(),
                            None,
                        )],
                        children: None,
                        span: directive.span.clone(),
                    }],
                );
            }

            // Copy any nested directives (like format)
            if let Some(children) = &directive.children {
                for (key, values) in children.directives.iter() {
                    directives.insert(key.clone(), values.clone());
                }
            }
        }
        "error_log" => {
            // error_log /path/to/error.log { ... } -> observability { provider file; error_log /path/to/error.log; ... }
            directives.insert(
                "provider".to_string(),
                vec![ServerConfigurationDirectiveEntry {
                    args: vec![ServerConfigurationValue::String("file".to_string(), None)],
                    children: None,
                    span: directive.span.clone(),
                }],
            );

            // If first arg is a string, it's the error_log path
            if let Some(path_value) = directive.args.first().and_then(|v| v.as_str()) {
                directives.insert(
                    "error_log".to_string(),
                    vec![ServerConfigurationDirectiveEntry {
                        args: vec![ServerConfigurationValue::String(
                            path_value.to_string(),
                            None,
                        )],
                        children: None,
                        span: directive.span.clone(),
                    }],
                );
            }
        }
        "console_log" => {
            // console_log { ... } -> observability { provider console; ... }
            directives.insert(
                "provider".to_string(),
                vec![ServerConfigurationDirectiveEntry {
                    args: vec![ServerConfigurationValue::String(
                        "console".to_string(),
                        None,
                    )],
                    children: None,
                    span: directive.span.clone(),
                }],
            );

            // Copy any nested directives (like format)
            if let Some(children) = &directive.children {
                for (key, values) in children.directives.iter() {
                    directives.insert(key.clone(), values.clone());
                }
            }
        }
        _ => {
            return Err(
                format!("Unknown observability alias directive: {}", directive_name).into(),
            );
        }
    }

    Ok(Some(ServerConfigurationBlock {
        directives: Arc::new(directives),
        matchers: HashMap::new(),
        span: directive.span.clone(),
    }))
}

/// Known observability alias directive names that can be transformed into observability blocks.
pub const OBSERVABILITY_ALIAS_DIRECTIVES: &[&str] = &["log", "error_log", "console_log"];

/// Extract observability configuration from a ServerConfigurationBlock.
/// This processes both explicit `observability` blocks and alias directives (log, error_log, console_log).
pub struct ObservabilityConfigExtractor<'a> {
    pub config: &'a ServerConfigurationBlock,
}

impl<'a> ObservabilityConfigExtractor<'a> {
    pub fn new(config: &'a ServerConfigurationBlock) -> Self {
        Self { config }
    }

    /// Get all observability blocks (both explicit and from aliases).
    /// Returns a vector of transformed ServerConfigurationBlock instances.
    pub fn extract_observability_blocks(
        &self,
    ) -> Result<Vec<ServerConfigurationBlock>, Box<dyn std::error::Error>> {
        let mut blocks = Vec::new();

        // Extract explicit observability blocks
        if let Some(observability_directives) = self.config.directives.get("observability") {
            for directive in observability_directives {
                // Check if enabled
                if directive
                    .args
                    .first()
                    .and_then(|a| a.as_boolean())
                    .map(|b| !b)
                    .unwrap_or(false)
                {
                    continue;
                }

                // Use children if present, otherwise create empty block
                if let Some(children) = &directive.children {
                    blocks.push(children.clone());
                } else {
                    blocks.push(ServerConfigurationBlock {
                        directives: Arc::new(HashMap::new()),
                        matchers: HashMap::new(),
                        span: directive.span.clone(),
                    });
                }
            }
        }

        // Extract alias directives
        for alias_name in OBSERVABILITY_ALIAS_DIRECTIVES {
            if let Some(alias_directives) = self.config.directives.get(*alias_name) {
                for directive in alias_directives {
                    if let Some(block) = transform_observability_alias(alias_name, directive)? {
                        blocks.push(block);
                    }
                }
            }
        }

        Ok(blocks)
    }

    /// Check if any observability configuration is present (explicit or aliases).
    pub fn has_observability(&self) -> bool {
        self.config.directives.contains_key("observability")
            || OBSERVABILITY_ALIAS_DIRECTIVES
                .iter()
                .any(|alias| self.config.directives.contains_key(*alias))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_directive(
        args: Vec<ServerConfigurationValue>,
        children: Option<ServerConfigurationBlock>,
    ) -> ServerConfigurationDirectiveEntry {
        ServerConfigurationDirectiveEntry {
            args,
            children,
            span: None,
        }
    }

    #[test]
    fn transform_log_alias_with_path() {
        let directive = create_directive(
            vec![ServerConfigurationValue::String(
                "/var/log/access.log".to_string(),
                None,
            )],
            None,
        );

        let result = transform_observability_alias("log", &directive)
            .expect("Should transform")
            .expect("Should not be disabled");

        assert_eq!(
            result.get_value("provider").unwrap().as_str().unwrap(),
            "file"
        );
        assert_eq!(
            result.get_value("access_log").unwrap().as_str().unwrap(),
            "/var/log/access.log"
        );
    }

    #[test]
    fn transform_log_alias_disabled() {
        let directive =
            create_directive(vec![ServerConfigurationValue::Boolean(false, None)], None);

        let result = transform_observability_alias("log", &directive).expect("Should transform");
        assert!(result.is_none());
    }

    #[test]
    fn transform_error_log_alias_with_path() {
        let directive = create_directive(
            vec![ServerConfigurationValue::String(
                "/var/log/error.log".to_string(),
                None,
            )],
            None,
        );

        let result = transform_observability_alias("error_log", &directive)
            .expect("Should transform")
            .expect("Should not be disabled");

        assert_eq!(
            result.get_value("provider").unwrap().as_str().unwrap(),
            "file"
        );
        assert_eq!(
            result.get_value("error_log").unwrap().as_str().unwrap(),
            "/var/log/error.log"
        );
    }

    #[test]
    fn transform_console_log_alias() {
        let directive = create_directive(vec![], None);

        let result = transform_observability_alias("console_log", &directive)
            .expect("Should transform")
            .expect("Should not be disabled");

        assert_eq!(
            result.get_value("provider").unwrap().as_str().unwrap(),
            "console"
        );
    }

    #[test]
    fn transform_console_log_alias_disabled() {
        let directive =
            create_directive(vec![ServerConfigurationValue::Boolean(false, None)], None);

        let result =
            transform_observability_alias("console_log", &directive).expect("Should transform");
        assert!(result.is_none());
    }
}
