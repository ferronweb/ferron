/// Validate argument types within a directive.
///
/// # Usage
///
/// ```ignore
/// // Single argument type
/// validate_args!(directive, [ServerConfigurationValue::String(_, _)]);
///
/// // Multiple argument types (positional)
/// validate_args!(directive, [
///     ServerConfigurationValue::String(_, _),
///     ServerConfigurationValue::Number(_, _)
/// ]);
///
/// // "Or" pattern - argument can be one of multiple types
/// validate_args!(directive, [
///     ServerConfigurationValue::Number(_, _) | ServerConfigurationValue::String(_, _)
/// ]);
///
/// // Multiple arguments with "or" patterns
/// validate_args!(directive, [
///     ServerConfigurationValue::String(_, _) | ServerConfigurationValue::Ident(_, _),
///     ServerConfigurationValue::Number(_, _) | ServerConfigurationValue::Boolean(_, _)
/// ]);
/// ```
#[macro_export]
macro_rules! validate_args {
    // Single pattern (may include "or" patterns like Type1 | Type2)
    ($directive:expr, [$pattern:pat $(if $guard:expr)?]) => {
        if !$directive.args.is_empty() && !matches!($directive.args[0], $pattern $(if $guard)?) {
            return Err($crate::config::validator::ConfigurationValidationError {
                inner: "Invalid directive: argument type mismatch at position 0".into(),
                span: $directive.span.clone(),
            });
        }
    };

    // Multiple patterns - use internal helper with counter
    ($directive:expr, [$($pattern:pat $(if $guard:expr)?),+]) => {
        $crate::validate_args!(@multi $directive, 0, [$($pattern $(if $guard)?),+])
    };

    // Internal: process multiple patterns with index counter
    (@multi $directive:expr, $idx:expr, [$pattern:pat $(if $guard:expr)?]) => {
        if !$directive.args.is_empty() && !matches!($directive.args[$idx], $pattern $(if $guard)?) {
            return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                "Invalid directive: argument type mismatch at position {}",
                $idx
            ).into(),
                span: $directive.span.clone(),
            });
        }
    };

    (@multi $directive:expr, $idx:expr, [$pattern:pat $(if $guard:expr)?, $($rest:tt)+]) => {
        if !$directive.args.is_empty() && !matches!($directive.args[$idx], $pattern $(if $guard)?) {
            return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                "Invalid directive: argument type mismatch at position {}",
                $idx
            ).into(),
                span: $directive.span.clone(),
            });
        }
        $crate::validate_args!(@multi $directive, $idx + 1, [$($rest)+])
    };

    // Check helper - returns true if patterns match, false otherwise (for use in "or" variations)
    (@check $directive:expr, [$($pattern:pat $(if $guard:expr)?),+]) => {
        $crate::validate_args!(@check_impl $directive, 0, [$($pattern $(if $guard)?),+])
    };

    (@check_impl $directive:expr, $idx:expr, [$pattern:pat $(if $guard:expr)?]) => {
        !$directive.args.is_empty() && matches!($directive.args[$idx], $pattern $(if $guard)?)
    };

    (@check_impl $directive:expr, $idx:expr, [$pattern:pat $(if $guard:expr)?, $($rest:tt)+]) => {
        !$directive.args.is_empty() && matches!($directive.args[$idx], $pattern $(if $guard)?) &&
        $crate::validate_args!(@check_impl $directive, $idx + 1, [$($rest)+])
    };
}
