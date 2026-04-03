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
            if let (Some(l), Some(r)) = (left_val, right_val) {
                r.split(',').any(|item| item.trim() == l)
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
}
