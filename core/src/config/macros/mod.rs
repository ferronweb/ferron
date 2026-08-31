//! Macros for configuration validation and building.
//!
//! These macros reduce boilerplate in [`ConfigurationValidator`](crate::config::validator::ConfigurationValidator)
//! implementations by providing declarative syntax for checking directive
//! structure, argument counts, and argument types.
//!
//! # Key macros
//!
//! | Macro | Purpose |
//! |---|---|
//! | [`validate_directive!`] | Validate a top-level directive (required or optional) |
//! | [`validate_nested!`] | Validate subdirectives within a block |
//! | [`validate_args!`] | Check argument types within a directive |
//! | [`check_unused_subdirectives!`] | Emit diagnostics for unrecognized subdirectives |
//! | [`require_directive!`] | Require a directive to exist (error if missing) |

#[macro_use]
mod args;
#[macro_use]
mod nested;

/// Validate a top-level directive in a configuration block.
///
/// # Usage
///
/// ```ignore
/// // Directive with no arguments
/// validate_directive!(config, used, runtime, no_args, {
///     // body runs for each directive instance
///     // `directive` variable is bound to ServerConfigurationDirectiveEntry
///     // `runtime` variable is bound to the children block (&ServerConfigurationBlock)
/// });
///
/// // Directive with exact argument count
/// validate_directive!(config, used, port, args(1), {
///     // validates arg count, body runs for each instance
/// });
///
/// // Directive with argument type patterns
/// validate_directive!(config, used, port, args(1) => [ServerConfigurationValue::Number(_, _)], {
///     // validates arg count and types
/// });
///
/// // Directive with minimum argument count
/// validate_directive!(config, used, listen, args(min = 1), {
///     // validates at least 1 argument
/// });
///
/// // Directive with maximum argument count
/// validate_directive!(config, used, options, args(max = 3), {
///     // validates at most 3 arguments
/// });
///
/// // Directive with argument range
/// validate_directive!(config, used, range, args(min = 1, max = 4), {
///     // validates between 1 and 4 arguments (inclusive)
/// });
///
/// // Directive with optional arguments (0 or more)
/// validate_directive!(config, used, flags, args(?), {
///     // any number of arguments including 0
/// });
///
/// // Directive with any number of arguments of a specific type
/// validate_directive!(config, used, items, args(*) => [ServerConfigurationValue::String(_, _)], {
///     // validates that all arguments (0 or more) are strings
/// });
///
/// // Optional directive (no error if missing)
/// validate_directive!(config, used, debug, optional, {
///     // only runs if directive exists
/// });
///
/// // Optional directive with argument validation
/// validate_directive!(config, used, timeout, optional args(1) => [ServerConfigurationValue::Number(_, _)], {
///     // only runs if directive exists and has correct arg type
/// });
///
/// // Optional directive with any number of arguments of a specific type
/// validate_directive!(config, used, tags, optional args(*) => [ServerConfigurationValue::String(_, _)], {
///     // only runs if directive exists, validates all args are strings
/// });
///
/// // Multiple variations with "or" - directive can have one of several signatures
/// validate_directive!(config, used, value,
///     args(1) => [ServerConfigurationValue::Number(_, _)]
///     | args(2) => [ServerConfigurationValue::String(_, _), ServerConfigurationValue::Number(_, _)]
///     | args(3) => [ServerConfigurationValue::String(_, _), ServerConfigurationValue::String(_, _), ServerConfigurationValue::Boolean(_, _)]
/// , {
///     // runs if directive matches any of the signatures
/// });
///
/// // "Or" operator with optional directive
/// validate_directive!(config, used, setting, optional
///     args(1) => [ServerConfigurationValue::Boolean(_, _)]
///     | args(2) => [ServerConfigurationValue::String(_, _), ServerConfigurationValue::Number(_, _)]
/// , {
///     // only runs if directive exists and matches one of the signatures
/// });
/// ```
#[macro_export]
macro_rules! validate_directive {
    // No arguments expected
    ($config:expr, $used:expr, $name:ident, no_args, $body:block) => {
        $crate::validate_directive!(@inner $config, $used, $name, exact 0, $body)
    };

    // Any number of arguments (0 or more) with type pattern validation - must come before args($count:expr)
    ($config:expr, $used:expr, $name:ident, args(*) => [$($pattern:pat),+], $body:block) => {
        if let Some(directives) = $config.directives.get(stringify!($name)) {
            $used.insert(stringify!($name).to_string());
            for directive in directives {
                for (idx, arg) in directive.args.iter().enumerate() {
                    if !matches!(arg, $($pattern)+) {
                        return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                            "Invalid directive '{}': argument type mismatch at position {}",
                            stringify!($name), idx
                        ).into(),
                span: $directive.span.clone(),
            });
                    }
                }
                let __empty = Default::default();
                let $name = directive.children.as_ref().unwrap_or(&__empty);
                $body
            }
        }
    };

    // Optional directive with any number of arguments and type pattern validation - must come before optional args($count:expr)
    ($config:expr, $used:expr, $name:ident, optional args(*) => [$($pattern:pat),+], $body:block) => {
        if let Some(directives) = $config.directives.get(stringify!($name)) {
            $used.insert(stringify!($name).to_string());
            for directive in directives {
                for (idx, arg) in directive.args.iter().enumerate() {
                    if !matches!(arg, $($pattern)+) {
                        return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                            "Invalid directive '{}': argument type mismatch at position {}",
                            stringify!($name), idx
                        ).into(),
                span: directive.span.clone(),
            });
                    }
                }
                let $name = directive.children.as_ref();
                $body
            }
        }
    };

    // Minimum argument count
    ($config:expr, $used:expr, $name:ident, args(min = $min:expr), $body:block) => {
        $crate::validate_directive!(@inner $config, $used, $name, min $min, $body)
    };

    // Maximum argument count
    ($config:expr, $used:expr, $name:ident, args(max = $max:expr), $body:block) => {
        $crate::validate_directive!(@inner $config, $used, $name, max $max, $body)
    };

    // Argument range (inclusive)
    ($config:expr, $used:expr, $name:ident, args(min = $min:expr, max = $max:expr), $body:block) => {
        $crate::validate_directive!(@inner_range $config, $used, $name, $min..=$max, $body)
    };

    // Exact argument count
    ($config:expr, $used:expr, $name:ident, args($count:expr), $body:block) => {
        $crate::validate_directive!(@inner $config, $used, $name, exact $count, $body)
    };

    // Exact argument count with type patterns
    ($config:expr, $used:expr, $name:ident, args($count:expr) => [$($pattern:pat),+], $body:block) => {
        $crate::validate_directive!(@inner $config, $used, $name, exact $count, [$($pattern),+], $body)
    };

    // Minimum argument count with type patterns
    ($config:expr, $used:expr, $name:ident, args(min = $min:expr) => [$($pattern:pat),+], $body:block) => {
        $crate::validate_directive!(@inner $config, $used, $name, min $min, [$($pattern),+], $body)
    };

    // Maximum argument count with type patterns
    ($config:expr, $used:expr, $name:ident, args(max = $max:expr) => [$($pattern:pat),+], $body:block) => {
        $crate::validate_directive!(@inner $config, $used, $name, max $max, [$($pattern),+], $body)
    };

    // Argument range with type patterns
    ($config:expr, $used:expr, $name:ident, args($range:expr) => [$($pattern:pat),+], $body:block) => {
        $crate::validate_directive!(@inner_range $config, $used, $name, $range, [$($pattern),+], $body)
    };

    // Optional directive with exact arg count and type patterns
    ($config:expr, $used:expr, $name:ident, optional args($count:expr) => [$($pattern:pat),+], $body:block) => {
        $crate::validate_directive!(@inner_optional $config, $used, $name, exact $count, [$($pattern),+], $body)
    };

    // Optional directive with minimum arg count and type patterns
    ($config:expr, $used:expr, $name:ident, optional args(min = $min:expr) => [$($pattern:pat),+], $body:block) => {
        $crate::validate_directive!(@inner_optional $config, $used, $name, min $min, [$($pattern),+], $body)
    };

    // Optional directive with maximum arg count and type patterns
    ($config:expr, $used:expr, $name:ident, optional args(max = $max:expr) => [$($pattern:pat),+], $body:block) => {
        $crate::validate_directive!(@inner_optional $config, $used, $name, max $max, [$($pattern),+], $body)
    };

    // Optional directive with arg range and type patterns
    ($config:expr, $used:expr, $name:ident, optional args($range:expr) => [$($pattern:pat),+], $body:block) => {
        $crate::validate_directive!(@inner_optional_range $config, $used, $name, $range, [$($pattern),+], $body)
    };

    // Optional arguments (0 or more, no validation)
    ($config:expr, $used:expr, $name:ident, args(?), $body:block) => {
        $crate::validate_directive!(@inner $config, $used, $name, any, $body)
    };

    // Optional directive (no error if missing) - no args
    ($config:expr, $used:expr, $name:ident, optional, $body:block) => {
        $crate::validate_directive!(@inner_optional $config, $used, $name, none, $body)
    };

    // Optional directive with arg count and type patterns
    ($config:expr, $used:expr, $name:ident, optional args($count:expr) => [$($pattern:pat),+], $body:block) => {
        $crate::validate_directive!(@inner_optional $config, $used, $name, exact $count, {
            $crate::validate_args!(directive, [$($pattern),+]);
            $body
        })
    };

    // Optional directive with minimum arg count
    ($config:expr, $used:expr, $name:ident, optional args(min = $min:expr), $body:block) => {
        $crate::validate_directive!(@inner_optional $config, $used, $name, min $min, $body)
    };

    // Optional directive with maximum arg count
    ($config:expr, $used:expr, $name:ident, optional args(max = $max:expr), $body:block) => {
        $crate::validate_directive!(@inner_optional $config, $used, $name, max $max, $body)
    };

    // Optional directive with arg range
    ($config:expr, $used:expr, $name:ident, optional args(min = $min:expr, max = $max:expr), $body:block) => {
        $crate::validate_directive!(@inner_optional_range $config, $used, $name, $min..=$max, $body)
    };

    // Optional directive with exact arg count
    ($config:expr, $used:expr, $name:ident, optional args($count:expr), $body:block) => {
        $crate::validate_directive!(@inner_optional $config, $used, $name, exact $count, $body)
    };

    // Optional directive with optional args
    ($config:expr, $used:expr, $name:ident, optional args(?), $body:block) => {
        $crate::validate_directive!(@inner_optional $config, $used, $name, any, $body)
    };

    // Multiple variations with "or" operator - required directive
    ($config:expr, $used:expr, $name:ident, args($count1:expr) => [$($pattern1:pat),+] $(| args($countN:expr) => [$($patternN:pat),+])+ , $body:block) => {
        if let Some(directives) = $config.directives.get(stringify!($name)) {
            $used.insert(stringify!($name).to_string());
            for directive in directives {
                let mut matched = false;
                if directive.args.len() == $count1 {
                    if $crate::validate_args!(@check directive, [$($pattern1),+]) {
                        matched = true;
                    }
                }
                $(
                    if !matched && directive.args.len() == $countN {
                        if $crate::validate_args!(@check directive, [$($patternN),+]) {
                            matched = true;
                        }
                    }
                )+
                if !matched {
                    return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                        "Invalid directive '{}': argument count or type mismatch (expected one of the valid signatures)",
                        stringify!($name)
                    ).into(),
                span: $directive.span.clone(),
            });
                }
                let __empty = Default::default();
                let $name = directive.children.as_ref().unwrap_or(&__empty);
                $body
            }
        }
    };

    // Multiple variations with "or" operator - optional directive
    ($config:expr, $used:expr, $name:ident, optional args($count1:expr) => [$($pattern1:pat),+] $(| args($countN:expr) => [$($patternN:pat),+])+ , $body:block) => {
        if let Some(directives) = $config.directives.get(stringify!($name)) {
            $used.insert(stringify!($name).to_string());
            for directive in directives {
                let mut matched = false;
                if directive.args.len() == $count1 {
                    if $crate::validate_args!(@check directive, [$($pattern1),+]) {
                        matched = true;
                    }
                }
                $(
                    if !matched && directive.args.len() == $countN {
                        if $crate::validate_args!(@check directive, [$($patternN),+]) {
                            matched = true;
                        }
                    }
                )+
                if !matched && !directive.args.is_empty() {
                    return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                        "Invalid directive '{}': argument count or type mismatch (expected one of the valid signatures)",
                        stringify!($name)
                    ).into(),
                span: directive.span.clone(),
            });
                }
                let __empty = Default::default();
                let $name = directive.children.as_ref().unwrap_or(&__empty);
                $body
            }
        }
    };

    // Internal implementation for required directives - exact count without patterns
    (@inner $config:expr, $used:expr, $name:ident, exact $count:expr, $body:block) => {
        if let Some(directives) = $config.directives.get(stringify!($name)) {
            $used.insert(stringify!($name).to_string());
            for directive in directives {
                if directive.args.len() != $count {
                    return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                        "Invalid directive '{}': expected {} argument(s), got {}",
                        stringify!($name), $count, directive.args.len()
                    ).into(),
                span: directive.span.clone(),
            });
                }
                let __empty = Default::default();
                let $name = directive.children.as_ref().unwrap_or(&__empty);
                $body
            }
        }
    };

    // Internal implementation for required directives - exact count with patterns
    (@inner $config:expr, $used:expr, $name:ident, exact $count:expr, [$($pattern:pat),+], $body:block) => {
        if let Some(directives) = $config.directives.get(stringify!($name)) {
            $used.insert(stringify!($name).to_string());
            for directive in directives {
                if directive.args.len() != $count {
                    return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                        "Invalid directive '{}': expected {} argument(s), got {}",
                        stringify!($name), $count, directive.args.len()
                    ).into(),
                span: directive.span.clone(),
            });
                }
                $crate::validate_args!(directive, [$($pattern),+]);
                let __empty = Default::default();
                let $name = directive.children.as_ref().unwrap_or(&__empty);
                $body
            }
        }
    };

    // Internal implementation - minimum args without patterns
    (@inner $config:expr, $used:expr, $name:ident, min $min:expr, $body:block) => {
        if let Some(directives) = $config.directives.get(stringify!($name)) {
            $used.insert(stringify!($name).to_string());
            for directive in directives {
                if directive.args.len() < $min {
                    return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                        "Invalid directive '{}': expected at least {} argument(s), got {}",
                        stringify!($name), $min, directive.args.len()
                    ).into(),
                span: $directive.span.clone(),
            });
                }
                let __empty = Default::default();
                let $name = directive.children.as_ref().unwrap_or(&__empty);
                $body
            }
        }
    };

    // Internal implementation - minimum args with patterns
    (@inner $config:expr, $used:expr, $name:ident, min $min:expr, [$($pattern:pat),+], $body:block) => {
        if let Some(directives) = $config.directives.get(stringify!($name)) {
            $used.insert(stringify!($name).to_string());
            for directive in directives {
                if directive.args.len() < $min {
                    return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                        "Invalid directive '{}': expected at least {} argument(s), got {}",
                        stringify!($name), $min, directive.args.len()
                    ).into(),
                span: $directive.span.clone(),
            });
                }
                $crate::validate_args!(directive, [$($pattern),+]);
                let __empty = Default::default();
                let $name = directive.children.as_ref().unwrap_or(&__empty);
                $body
            }
        }
    };

    // Internal implementation - maximum args without patterns
    (@inner $config:expr, $used:expr, $name:ident, max $max:expr, $body:block) => {
        if let Some(directives) = $config.directives.get(stringify!($name)) {
            $used.insert(stringify!($name).to_string());
            for directive in directives {
                if directive.args.len() > $max {
                    return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                        "Invalid directive '{}': expected at most {} argument(s), got {}",
                        stringify!($name), $max, directive.args.len()
                    ).into(),
                span: $directive.span.clone(),
            });
                }
                let __empty = Default::default();
                let $name = directive.children.as_ref().unwrap_or(&__empty);
                $body
            }
        }
    };

    // Internal implementation - maximum args with patterns
    (@inner $config:expr, $used:expr, $name:ident, max $max:expr, [$($pattern:pat),+], $body:block) => {
        if let Some(directives) = $config.directives.get(stringify!($name)) {
            $used.insert(stringify!($name).to_string());
            for directive in directives {
                if directive.args.len() > $max {
                    return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                        "Invalid directive '{}': expected at most {} argument(s), got {}",
                        stringify!($name), $max, directive.args.len()
                    ).into(),
                span: $directive.span.clone(),
            });
                }
                $crate::validate_args!(directive, [$($pattern),+]);
                let __empty = Default::default();
                let $name = directive.children.as_ref().unwrap_or(&__empty);
                $body
            }
        }
    };

    // Internal implementation - range without patterns
    (@inner_range $config:expr, $used:expr, $name:ident, $range:expr, $body:block) => {
        if let Some(directives) = $config.directives.get(stringify!($name)) {
            $used.insert(stringify!($name).to_string());
            for directive in directives {
                if !$range.contains(&directive.args.len()) {
                    return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                        "Invalid directive '{}': expected {}-{} argument(s), got {}",
                        stringify!($name), $range.min().unwrap_or(0), $range.max().unwrap_or(0), directive.args.len()
                    ).into(),
                span: $directive.span.clone(),
            });
                }
                let __empty = Default::default();
                let $name = directive.children.as_ref().unwrap_or(&__empty);
                $body
            }
        }
    };

    // Internal implementation - range with patterns
    (@inner_range $config:expr, $used:expr, $name:ident, $range:expr, [$($pattern:pat),+], $body:block) => {
        if let Some(directives) = $config.directives.get(stringify!($name)) {
            $used.insert(stringify!($name).to_string());
            for directive in directives {
                if !$range.contains(&directive.args.len()) {
                    return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                        "Invalid directive '{}': expected {}-{} argument(s), got {}",
                        stringify!($name), $range.min().unwrap_or(0), $range.max().unwrap_or(0), directive.args.len()
                    ).into(),
                span: $directive.span.clone(),
            });
                }
                $crate::validate_args!(directive, [$($pattern),+]);
                let __empty = Default::default();
                let $name = directive.children.as_ref().unwrap_or(&__empty);
                $body
            }
        }
    };

    // Internal implementation - any number of args
    (@inner $config:expr, $used:expr, $name:ident, any, $body:block) => {
        if let Some(directives) = $config.directives.get(stringify!($name)) {
            $used.insert(stringify!($name).to_string());
            for directive in directives {
                let __empty = Default::default();
                let $name = directive.children.as_ref().unwrap_or(&__empty);
                $body
            }
        }
    };

    // Internal implementation for optional directives - none
    (@inner_optional $config:expr, $used:expr, $name:ident, none, $body:block) => {
        if let Some(directives) = $config.directives.get(stringify!($name)) {
            $used.insert(stringify!($name).to_string());
            for directive in directives {
                let $name = directive.children.as_ref();
                $body
            }
        }
    };

    // Internal implementation for optional directives - exact count without patterns
    (@inner_optional $config:expr, $used:expr, $name:ident, exact $count:expr, $body:block) => {
        if let Some(directives) = $config.directives.get(stringify!($name)) {
            $used.insert(stringify!($name).to_string());
            for directive in directives {
                if directive.args.len() != 0 && directive.args.len() != $count {
                    return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                        "Invalid directive '{}': expected {} argument(s), got {}",
                        stringify!($name), $count, directive.args.len()
                    ).into(),
                span: $directive.span.clone(),
            });
                }
                let __empty = Default::default();
                let $name = directive.children.as_ref().unwrap_or(&__empty);
                $body
            }
        }
    };

    // Internal implementation for optional directives - exact count with patterns
    (@inner_optional $config:expr, $used:expr, $name:ident, exact $count:expr, [$($pattern:pat),+], $body:block) => {
        if let Some(directives) = $config.directives.get(stringify!($name)) {
            $used.insert(stringify!($name).to_string());
            for directive in directives {
                if directive.args.len() != 0 {
                    if directive.args.len() != $count {
                        return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                            "Invalid directive '{}': expected {} argument(s), got {}",
                            stringify!($name), $count, directive.args.len()
                        ).into(),
                span: directive.span.clone(),
            });
                    }
                    $crate::validate_args!(directive, [$($pattern),+]);
                }
                let __empty = Default::default();
                let $name = directive.children.as_ref().unwrap_or(&__empty);
                $body
            }
        }
    };

    // Internal implementation for optional directives - min without patterns
    (@inner_optional $config:expr, $used:expr, $name:ident, min $min:expr, $body:block) => {
        if let Some(directives) = $config.directives.get(stringify!($name)) {
            $used.insert(stringify!($name).to_string());
            for directive in directives {
                if directive.args.len() != 0 && directive.args.len() < $min {
                    return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                        "Invalid directive '{}': expected at least {} argument(s), got {}",
                        stringify!($name), $min, directive.args.len()
                    ).into(),
                span: $directive.span.clone(),
            });
                }
                let __empty = Default::default();
                let $name = directive.children.as_ref().unwrap_or(&__empty);
                $body
            }
        }
    };

    // Internal implementation for optional directives - min with patterns
    (@inner_optional $config:expr, $used:expr, $name:ident, min $min:expr, [$($pattern:pat),+], $body:block) => {
        if let Some(directives) = $config.directives.get(stringify!($name)) {
            $used.insert(stringify!($name).to_string());
            for directive in directives {
                if directive.args.len() != 0 {
                    directive.args.len() < $min {
                        return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                            "Invalid directive '{}': expected at least {} argument(s), got {}",
                            stringify!($name), $min, directive.args.len()
                        ).into(),
                span: $directive.span.clone(),
            });
                    }
                    $crate::validate_args!(directive, [$($pattern),+]);
                }
                let __empty = Default::default();
                let $name = directive.children.as_ref().unwrap_or(&__empty);
                $body
            }
        }
    };

    // Internal implementation for optional directives - max without patterns
    (@inner_optional $config:expr, $used:expr, $name:ident, max $max:expr, $body:block) => {
        if let Some(directives) = $config.directives.get(stringify!($name)) {
            $used.insert(stringify!($name).to_string());
            for directive in directives {
                if directive.args.len() != 0 && directive.args.len() > $max {
                    return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                        "Invalid directive '{}': expected at most {} argument(s), got {}",
                        stringify!($name), $max, directive.args.len()
                    ).into(),
                span: $directive.span.clone(),
            });
                }
                let __empty = Default::default();
                let $name = directive.children.as_ref().unwrap_or(&__empty);
                $body
            }
        }
    };

    // Internal implementation for optional directives - max with patterns
    (@inner_optional $config:expr, $used:expr, $name:ident, max $max:expr, [$($pattern:pat),+], $body:block) => {
        if let Some(directives) = $config.directives.get(stringify!($name)) {
            $used.insert(stringify!($name).to_string());
            for directive in directives {
                if directive.args.len() != 0 {
                    if directive.args.len() > $max {
                        return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                            "Invalid directive '{}': expected at most {} argument(s), got {}",
                            stringify!($name), $max, directive.args.len()
                        ).into(),
                span: $directive.span.clone(),
            });
                    }
                    $crate::validate_args!(directive, [$($pattern),+]);
                }
                let __empty = Default::default();
                let $name = directive.children.as_ref().unwrap_or(&__empty);
                $body
            }
        }
    };

    // Internal implementation for optional directives - range without patterns
    (@inner_optional_range $config:expr, $used:expr, $name:ident, $range:expr, $body:block) => {
        if let Some(directives) = $config.directives.get(stringify!($name)) {
            $used.insert(stringify!($name).to_string());
            for directive in directives {
                if directive.args.len() != 0 && !$range.contains(&directive.args.len()) {
                    return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                        "Invalid directive '{}': expected {} argument(s), got {}",
                        stringify!($name), $range, directive.args.len()
                    ).into(),
                span: $directive.span.clone(),
            });
                }
                let __empty = Default::default();
                let $name = directive.children.as_ref().unwrap_or(&__empty);
                $body
            }
        }
    };

    // Internal implementation for optional directives - range with patterns
    (@inner_optional_range $config:expr, $used:expr, $name:ident, $range:expr, [$($pattern:pat),+], $body:block) => {
        if let Some(directives) = $config.directives.get(stringify!($name)) {
            $used.insert(stringify!($name).to_string());
            for directive in directives {
                if directive.args.len() != 0 {
                    if !$range.contains(&directive.args.len()) {
                        return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                            "Invalid directive '{}': expected {} argument(s), got {}",
                            stringify!($name), $range, directive.args.len()
                        ).into(),
                span: $directive.span.clone(),
            });
                    }
                    $crate::validate_args!(directive, [$($pattern),+]);
                }
                let __empty = Default::default();
                let $name = directive.children.as_ref().unwrap_or(&__empty);
                $body
            }
        }
    };

    // Internal implementation for optional directives - any
    (@inner_optional $config:expr, $used:expr, $name:ident, any, $body:block) => {
        if let Some(directives) = $config.directives.get(stringify!($name)) {
            $used.insert(stringify!($name).to_string());
            for directive in directives {
                let $name = directive.children.as_ref();
                $body
            }
        }
    };

    // Internal implementation for optional directives - all arguments must match pattern (any count)
    (@inner_optional $config:expr, $used:expr, $name:ident, all_patterns [$($pattern:pat),+], $body:block) => {
        if let Some(directives) = $config.directives.get(stringify!($name)) {
            $used.insert(stringify!($name).to_string());
            for directive in directives {
                $crate::validate_directive!(@check_all_args directive, 0, [$($pattern),+], $name);
                let $name = directive.children.as_ref();
                $body
            }
        }
    };
}

/// Emit `UnknownDirective` diagnostics for directives in a block that were not tracked as used.
///
/// Use after validating all known subdirectives in a nested block to catch unrecognized ones.
///
/// # Usage
///
/// ```ignore
/// let mut local = std::collections::HashSet::new();
/// validate_nested!(block, used(local), known_directive, args(1) => [Type]);
/// // ... more validate_nested! calls
/// check_unused_subdirectives!(block, local, diagnostics, scope);
/// ```
///
/// `diagnostics` must be `&mut Vec<ConfigurationValidatorDiagnostic>`, `scope` is `Option<String>`.
#[macro_export]
macro_rules! check_unused_subdirectives {
    ($block:expr, $used:expr, $diagnostics:expr, $scope:expr) => {
        for (directive_name, span) in $block.directives
            .iter()
            .filter(|d| !$used.contains(d.0))
            .flat_map(|d| d.1.iter().map(|s| (d.0.clone(), s.span.clone()))) {
            $diagnostics.push(
                $crate::config::validator::ConfigurationValidatorDiagnostic {
                    kind: $crate::config::validator::ConfigurationValidatorDiagnosticKind::UnknownDirective,
                    message: format!("`{directive_name}` is unused in the block"),
                    span: span.or($block.span.clone()),
                    scope: $scope.clone(),
                }
            );
        }
    };
}

/// Require a directive to exist (errors if missing).
///
/// # Usage
///
/// ```ignore
/// let directives = require_directive!(config, used, "port", "port directive is required");
/// ```
#[macro_export]
macro_rules! require_directive {
    ($config:expr, $used:expr, $name:literal, $error:literal) => {
        $used.insert($name.to_string());
        let directives = $config.directives.get(stringify!($name)).ok_or($error)?;
        directives
    };
}
