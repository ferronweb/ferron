use std::sync::Arc;

use fancy_regex::Regex;
use ferron_core::config::{
    ServerConfigurationMatcherExpr, ServerConfigurationMatcherOperand,
    ServerConfigurationMatcherOperator,
};
use ferron_http::variables::resolve_variable;

use super::types::ResolverVariables;

/// A matcher expression with pre-compiled regex patterns for efficient evaluation.
///
/// This struct caches compiled regex patterns to avoid recompiling them on every evaluation.
/// Regexes are compiled at configuration insertion time, not at evaluation time.
/// Only Regex and NotRegex operations use compiled patterns; other operators work with string values.
#[derive(Debug, Clone)]
pub struct CompiledMatcherExpr {
    /// The original matcher expression
    pub expr: ServerConfigurationMatcherExpr,
    /// Compiled regex for the right operand if it's a Regex/NotRegex operation with a static pattern
    pub compiled_regex: Option<Arc<Regex>>,
}

impl CompiledMatcherExpr {
    /// Create a new compiled matcher expression, pre-compiling regex if needed
    ///
    /// Returns `Err` if regex compilation fails at insertion time.
    pub fn new(expr: ServerConfigurationMatcherExpr) -> Result<Self, String> {
        let compiled_regex = if matches!(
            expr.op,
            ServerConfigurationMatcherOperator::Regex
                | ServerConfigurationMatcherOperator::NotRegex
        ) {
            // Extract the regex pattern from the right operand
            let pattern = match &expr.right {
                ServerConfigurationMatcherOperand::String(s) => Some(s.clone()),
                ServerConfigurationMatcherOperand::Identifier(_name) => {
                    // For identifiers, pattern is dynamic; will be compiled at runtime
                    None
                }
                ServerConfigurationMatcherOperand::Integer(n) => Some(n.to_string()),
                ServerConfigurationMatcherOperand::Float(f) => Some(f.to_string()),
            };

            if let Some(pattern) = pattern {
                match Regex::new(&pattern) {
                    Ok(regex) => Some(Arc::new(regex)),
                    Err(e) => return Err(format!("Invalid regex pattern '{}': {}", pattern, e)),
                }
            } else {
                None
            }
        } else {
            None
        };

        Ok(Self {
            expr,
            compiled_regex,
        })
    }
}

/// Evaluate a collection of conditional expressions with AND logic (all must match).
pub fn evaluate_matcher_conditions(
    exprs: &[CompiledMatcherExpr],
    variables: &ResolverVariables,
) -> bool {
    exprs
        .iter()
        .all(|expr| evaluate_matcher_condition(expr, variables))
}

/// Evaluate a single conditional matcher expression with given variables.
pub fn evaluate_matcher_condition(
    compiled_expr: &CompiledMatcherExpr,
    variables: &ResolverVariables,
) -> bool {
    let expr = &compiled_expr.expr;
    let left_val = resolve_matcher_operand(&expr.left, variables);
    let right_val = resolve_matcher_operand(&expr.right, variables);

    match &expr.op {
        ServerConfigurationMatcherOperator::Eq => left_val == right_val,
        ServerConfigurationMatcherOperator::NotEq => left_val != right_val,
        ServerConfigurationMatcherOperator::Regex => {
            if let Some(left) = left_val {
                if let Some(regex) = &compiled_expr.compiled_regex {
                    regex.is_match(&left).unwrap_or(false)
                } else if let Some(right) = right_val {
                    match Regex::new(&right) {
                        Ok(regex) => regex.is_match(&left).unwrap_or(false),
                        Err(_) => false,
                    }
                } else {
                    false
                }
            } else {
                false
            }
        }
        ServerConfigurationMatcherOperator::NotRegex => {
            if let Some(left) = left_val {
                if let Some(regex) = &compiled_expr.compiled_regex {
                    !regex.is_match(&left).unwrap_or(false)
                } else if let Some(right) = right_val {
                    match Regex::new(&right) {
                        Ok(regex) => !regex.is_match(&left).unwrap_or(false),
                        Err(_) => true,
                    }
                } else {
                    true
                }
            } else {
                true
            }
        }
        ServerConfigurationMatcherOperator::In => {
            // Accept-Language header matching: check if left value is in the parsed Accept-Language header
            if let (Some(left), Some(right)) = (left_val, right_val) {
                // Check if right looks like an Accept-Language header (contains q= or multiple comma-separated values)
                let is_accept_language = right.contains("q=")
                    || (right.contains(',') && !right.split(',').any(|s| s.trim().is_empty()));

                if is_accept_language {
                    // Parse as Accept-Language header with q-values
                    let accepted_languages: Vec<String> =
                        ferron_http::util::parse_q_value_header::parse_q_value_header(&right);
                    accepted_languages.iter().any(|lang| {
                        lang.eq_ignore_ascii_case(&left)
                            || lang
                                .split_once('-')
                                .map(|(base, _): (&str, &str)| base.eq_ignore_ascii_case(&left))
                                .unwrap_or(false)
                    })
                } else {
                    // Fallback to simple comma-separated list matching
                    right
                        .split(',')
                        .any(|item| item.trim().eq_ignore_ascii_case(&left))
                }
            } else {
                false
            }
        }
    }
}

/// Resolve the string value of a matcher operand from variables or literals.
pub fn resolve_matcher_operand(
    operand: &ServerConfigurationMatcherOperand,
    variables: &ResolverVariables,
) -> Option<String> {
    match operand {
        ServerConfigurationMatcherOperand::Identifier(name) => {
            resolve_variable(name, &variables.0, &variables.1)
        }
        ServerConfigurationMatcherOperand::String(s) => Some(s.clone()),
        ServerConfigurationMatcherOperand::Integer(n) => Some(n.to_string()),
        ServerConfigurationMatcherOperand::Float(f) => Some(f.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::config::ServerConfigurationMatcherExpr;
    use ferron_http::HttpRequest;
    use std::collections::HashMap;

    #[test]
    fn test_regex_matcher_expr_matching() {
        let expr = ServerConfigurationMatcherExpr {
            left: ServerConfigurationMatcherOperand::String("test123".to_string()),
            op: ServerConfigurationMatcherOperator::Regex,
            right: ServerConfigurationMatcherOperand::String(r"test\d+".to_string()),
        };

        let compiled = CompiledMatcherExpr::new(expr).unwrap();

        let req = HttpRequest::default();
        let variables = (req, HashMap::new());

        assert!(evaluate_matcher_condition(&compiled, &variables));
    }

    #[test]
    fn test_not_regex_matcher_expr() {
        let expr = ServerConfigurationMatcherExpr {
            left: ServerConfigurationMatcherOperand::String("test".to_string()),
            op: ServerConfigurationMatcherOperator::NotRegex,
            right: ServerConfigurationMatcherOperand::String(r"\d+".to_string()),
        };

        let compiled = CompiledMatcherExpr::new(expr).unwrap();

        let req = HttpRequest::default();
        let variables = (req, HashMap::new());

        assert!(evaluate_matcher_condition(&compiled, &variables));
    }

    #[test]
    fn test_fancy_regex_features() {
        let expr = ServerConfigurationMatcherExpr {
            left: ServerConfigurationMatcherOperand::String("foobar".to_string()),
            op: ServerConfigurationMatcherOperator::Regex,
            right: ServerConfigurationMatcherOperand::String(r"(foo|baz).*".to_string()),
        };

        let compiled = CompiledMatcherExpr::new(expr).unwrap();

        let req = HttpRequest::default();
        let variables = (req, HashMap::new());

        assert!(evaluate_matcher_condition(&compiled, &variables));
    }

    #[test]
    fn test_in_operator_accept_language_with_q_values() {
        let expr = ServerConfigurationMatcherExpr {
            left: ServerConfigurationMatcherOperand::String("en".to_string()),
            op: ServerConfigurationMatcherOperator::In,
            right: ServerConfigurationMatcherOperand::String(
                "en-US; q=0.8, fr-FR; q=0.5, de; q=0.3".to_string(),
            ),
        };

        let compiled = CompiledMatcherExpr::new(expr).unwrap();
        let req = HttpRequest::default();
        let variables = (req, HashMap::new());

        assert!(evaluate_matcher_condition(&compiled, &variables));
    }

    #[test]
    fn test_in_operator_accept_language_base_language_match() {
        let expr = ServerConfigurationMatcherExpr {
            left: ServerConfigurationMatcherOperand::String("en".to_string()),
            op: ServerConfigurationMatcherOperator::In,
            right: ServerConfigurationMatcherOperand::String(
                "en-US; q=0.8, fr-FR; q=0.5".to_string(),
            ),
        };

        let compiled = CompiledMatcherExpr::new(expr).unwrap();
        let req = HttpRequest::default();
        let variables = (req, HashMap::new());

        // Should match "en" from "en-US" base language
        assert!(evaluate_matcher_condition(&compiled, &variables));
    }

    #[test]
    fn test_in_operator_accept_language_no_match() {
        let expr = ServerConfigurationMatcherExpr {
            left: ServerConfigurationMatcherOperand::String("zh".to_string()),
            op: ServerConfigurationMatcherOperator::In,
            right: ServerConfigurationMatcherOperand::String(
                "en-US; q=0.8, fr-FR; q=0.5".to_string(),
            ),
        };

        let compiled = CompiledMatcherExpr::new(expr).unwrap();
        let req = HttpRequest::default();
        let variables = (req, HashMap::new());

        assert!(!evaluate_matcher_condition(&compiled, &variables));
    }

    #[test]
    fn test_in_operator_accept_language_case_insensitive() {
        let expr = ServerConfigurationMatcherExpr {
            left: ServerConfigurationMatcherOperand::String("EN".to_string()),
            op: ServerConfigurationMatcherOperator::In,
            right: ServerConfigurationMatcherOperand::String(
                "en-US; q=0.8, fr-FR; q=0.5".to_string(),
            ),
        };

        let compiled = CompiledMatcherExpr::new(expr).unwrap();
        let req = HttpRequest::default();
        let variables = (req, HashMap::new());

        assert!(evaluate_matcher_condition(&compiled, &variables));
    }

    #[test]
    fn test_in_operator_simple_comma_separated_list() {
        let expr = ServerConfigurationMatcherExpr {
            left: ServerConfigurationMatcherOperand::String("api".to_string()),
            op: ServerConfigurationMatcherOperator::In,
            right: ServerConfigurationMatcherOperand::String("web,api,admin".to_string()),
        };

        let compiled = CompiledMatcherExpr::new(expr).unwrap();
        let req = HttpRequest::default();
        let variables = (req, HashMap::new());

        assert!(evaluate_matcher_condition(&compiled, &variables));
    }

    #[test]
    fn test_in_operator_simple_comma_separated_list_no_match() {
        let expr = ServerConfigurationMatcherExpr {
            left: ServerConfigurationMatcherOperand::String("guest".to_string()),
            op: ServerConfigurationMatcherOperator::In,
            right: ServerConfigurationMatcherOperand::String("web,api,admin".to_string()),
        };

        let compiled = CompiledMatcherExpr::new(expr).unwrap();
        let req = HttpRequest::default();
        let variables = (req, HashMap::new());

        assert!(!evaluate_matcher_condition(&compiled, &variables));
    }

    #[test]
    fn test_in_operator_accept_language_without_q_values() {
        let expr = ServerConfigurationMatcherExpr {
            left: ServerConfigurationMatcherOperand::String("fr".to_string()),
            op: ServerConfigurationMatcherOperator::In,
            right: ServerConfigurationMatcherOperand::String("en-US, fr-FR, de-DE".to_string()),
        };

        let compiled = CompiledMatcherExpr::new(expr).unwrap();
        let req = HttpRequest::default();
        let variables = (req, HashMap::new());

        assert!(evaluate_matcher_condition(&compiled, &variables));
    }
}
