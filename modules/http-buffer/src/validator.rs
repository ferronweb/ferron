//! Configuration validator for `buffer_request` and `buffer_response` directives.
//!
//! Validates that both directives, if present, contain either an integer
//! (buffer size in bytes) or `#null` (disabled).
use std::collections::HashSet;

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::config::ServerConfigurationBlock;
use ferron_core::validate_directive;

/// Validator for HTTP buffer configuration blocks.
#[derive(Default)]
pub struct HttpBufferConfigurationValidator;

impl ConfigurationValidator for HttpBufferConfigurationValidator {
    fn validate_block(
        &self,
        config: &ServerConfigurationBlock,
        used_directives: &mut HashSet<String>,
        _is_global: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        validate_directive!(config, used_directives, buffer_request, optional
            args(1) => [ferron_core::config::ServerConfigurationValue::Number(_, _)], {});

        validate_directive!(config, used_directives, buffer_response, optional
            args(1) => [ferron_core::config::ServerConfigurationValue::Number(_, _)], {});

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::config::{ServerConfigurationDirectiveEntry, ServerConfigurationValue};
    use std::collections::HashMap as StdHashMap;
    use std::sync::Arc;

    fn make_value_number(n: i64) -> ServerConfigurationValue {
        ServerConfigurationValue::Number(n, None)
    }

    fn make_block_with_directives(
        directives: Vec<(&str, Vec<ServerConfigurationValue>)>,
    ) -> ServerConfigurationBlock {
        let mut d = StdHashMap::new();
        for (name, args) in directives {
            d.insert(
                name.to_string(),
                vec![ServerConfigurationDirectiveEntry {
                    args,
                    children: None,
                    span: None,
                }],
            );
        }
        ServerConfigurationBlock {
            directives: Arc::new(d),
            matchers: StdHashMap::new(),
            span: None,
        }
    }

    #[test]
    fn accepts_valid_buffer_request() {
        let block =
            make_block_with_directives(vec![("buffer_request", vec![make_value_number(8192)])]);
        let mut used = HashSet::new();
        let validator = HttpBufferConfigurationValidator;
        assert!(validator.validate_block(&block, &mut used, false).is_ok());
        assert!(used.contains("buffer_request"));
    }

    #[test]
    fn accepts_valid_buffer_response() {
        let block =
            make_block_with_directives(vec![("buffer_response", vec![make_value_number(65536)])]);
        let mut used = HashSet::new();
        let validator = HttpBufferConfigurationValidator;
        assert!(validator.validate_block(&block, &mut used, false).is_ok());
        assert!(used.contains("buffer_response"));
    }

    #[test]
    fn accepts_both_directives() {
        let block = make_block_with_directives(vec![
            ("buffer_request", vec![make_value_number(8192)]),
            ("buffer_response", vec![make_value_number(65536)]),
        ]);
        let mut used = HashSet::new();
        let validator = HttpBufferConfigurationValidator;
        assert!(validator.validate_block(&block, &mut used, false).is_ok());
        assert!(used.contains("buffer_request"));
        assert!(used.contains("buffer_response"));
    }

    #[test]
    fn rejects_wrong_arg_count() {
        let block = make_block_with_directives(vec![(
            "buffer_request",
            vec![make_value_number(8192), make_value_number(65536)],
        )]);
        let mut used = HashSet::new();
        let validator = HttpBufferConfigurationValidator;
        let err = validator
            .validate_block(&block, &mut used, false)
            .unwrap_err();
        assert!(err.to_string().contains("expected 1 argument"));
    }

    #[test]
    fn rejects_string_value() {
        let block = make_block_with_directives(vec![(
            "buffer_request",
            vec![ServerConfigurationValue::String("8192".to_string(), None)],
        )]);
        let mut used = HashSet::new();
        let validator = HttpBufferConfigurationValidator;
        let err = validator
            .validate_block(&block, &mut used, false)
            .unwrap_err();
        assert!(err.to_string().contains("argument type mismatch"));
    }

    #[test]
    fn skips_block_without_buffer_directives() {
        let block = make_block_with_directives(vec![]);
        let mut used = HashSet::new();
        let validator = HttpBufferConfigurationValidator;
        assert!(validator.validate_block(&block, &mut used, false).is_ok());
        assert!(!used.contains("buffer_request"));
        assert!(!used.contains("buffer_response"));
    }
}
