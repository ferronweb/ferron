/// Validate nested subdirectives within a configuration block.
///
/// # Usage
///
/// ```ignore
/// // Single subdirective with exact arg count and type pattern
/// validate_nested!(block, io_uring, args(1) => [ServerConfigurationValue::Boolean(_, _)]);
///
/// // Subdirective with multiple argument types (positional)
/// validate_nested!(block, options, args(2) => [
///     ServerConfigurationValue::String(_, _),
///     ServerConfigurationValue::Number(_, _)
/// ]);
///
/// // Subdirective with "or" pattern - argument can be one of multiple types
/// validate_nested!(block, value, args(1) => [
///     ServerConfigurationValue::Number(_, _) | ServerConfigurationValue::String(_, _)
/// ]);
///
/// // Subdirective with minimum arg count and type patterns
/// validate_nested!(block, items, args(min = 1) => [ServerConfigurationValue::String(_, _)]);
///
/// // Subdirective with maximum arg count and type patterns
/// validate_nested!(block, flags, args(max = 3) => [ServerConfigurationValue::Boolean(_, _)]);
///
/// // Subdirective with arg range and type patterns
/// validate_nested!(block, range, args(min = 1, max = 4) => [ServerConfigurationValue::Number(_, _)]);
///
/// // Subdirective with any number of args and type pattern
/// validate_nested!(block, items, args(?) => [ServerConfigurationValue::String(_, _)]);
///
/// // Subdirective with any number of arguments of a specific type
/// validate_nested!(block, tags, args(*) => [ServerConfigurationValue::String(_, _)]);
///
/// // Subdirective with nested block for deeper nesting
/// validate_nested!(block, pool, {
///     validate_nested!(pool, size, args(1) => [ServerConfigurationValue::Number(_, _)]);
/// });
///
/// // Subdirective with just existence check
/// validate_nested!(block, debug);
///
/// // Multiple variations with "or" - subdirective can have one of several signatures
/// validate_nested!(block, value,
///     args(1) => [ServerConfigurationValue::Number(_, _)]
///     | args(2) => [ServerConfigurationValue::String(_, _), ServerConfigurationValue::Number(_, _)]
///     | args(3) => [ServerConfigurationValue::String(_, _), ServerConfigurationValue::String(_, _), ServerConfigurationValue::Boolean(_, _)]
/// );
///
/// // Optional subdirective - no error if missing
/// validate_nested!(block, debug, optional);
///
/// // Optional subdirective with argument validation
/// validate_nested!(block, setting, optional args(1) => [ServerConfigurationValue::Boolean(_, _)]);
///
/// // Optional subdirective with multiple variations
/// validate_nested!(block, value, optional
///     args(1) => [ServerConfigurationValue::Number(_, _)]
///     | args(2) => [ServerConfigurationValue::String(_, _), ServerConfigurationValue::Number(_, _)]
/// );
///
/// // Optional subdirective with any number of arguments of a specific type
/// validate_nested!(block, tags, optional args(*) => [ServerConfigurationValue::String(_, _)]);
/// ```
#[macro_export]
macro_rules! validate_nested {
    // Tracking variant — marks the subdirective name as used, then delegates to the non-tracking variant.
    // Usage: validate_nested!(block, used(ctx.used_directives), name, args(1) => [Type]);
    ($block:expr, used($used:expr), $name:ident, $($rest:tt)*) => {
        {
            $used.insert(stringify!($name).to_string());
            $crate::validate_nested!($block, $name, $($rest)*)
        }
    };

    // Any number of arguments with type pattern validation (array syntax) - must come before args($count:expr)
    ($block:expr, $name:ident, args(*) => [$($pattern:pat $(if $guard:expr)?),+]) => {
        if let Some(directives) = $block.directives.get(stringify!($name)) {
            for directive in directives {
                for (idx, arg) in directive.args.iter().enumerate() {
                    if !matches!(arg, $($pattern $(if $guard)?)+) {
                        return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                            "Invalid directive '{}': invalid type for '{}' subdirective at position {}",
                            stringify!($block), stringify!($name), idx
                        ).into(),
                span: directive.span.clone(),
            });
                    }
                }
            }
        }
    };

    // Single subdirective with exact arg count and type patterns (array syntax)
    ($block:expr, $name:ident, args($count:expr) => [$($pattern:pat $(if $guard:expr)?),+]) => {
        if let Some(directives) = $block.directives.get(stringify!($name)) {
            for directive in directives {
                if directive.args.len() != $count {
                    return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                        "Invalid directive '{}': expected {} argument(s) in '{}' subdirective, got {}",
                        stringify!($block), $count, stringify!($name), directive.args.len()
                    ).into(),
                span: directive.span.clone(),
            });
                }
                $crate::validate_nested!(@check_args $block, directive, [$($pattern $(if $guard)?),+], $name);
            }
        }
    };

    // Single subdirective with minimum arg count and type patterns (array syntax)
    ($block:expr, $name:ident, args(min = $min:expr) => [$($pattern:pat $(if $guard:expr)?),+]) => {
        if let Some(directives) = $block.directives.get(stringify!($name)) {
            for directive in directives {
                if directive.args.len() < $min {
                    return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                        "Invalid directive '{}': expected at least {} argument(s) in '{}' subdirective, got {}",
                        stringify!($block), $min, $name, directive.args.len()
                    ).into(),
                span: $directive.span.clone(),
            });
                }
                $crate::validate_nested!(@check_args $block, directive, [$($pattern $(if $guard)?),+], $name);
            }
        }
    };

    // Single subdirective with maximum arg count and type patterns (array syntax)
    ($block:expr, $name:ident, args(max = $max:expr) => [$($pattern:pat $(if $guard:expr)?),+]) => {
        if let Some(directives) = $block.directives.get(stringify!($name)) {
            for directive in directives {
                if directive.args.len() > $max {
                    return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                        "Invalid directive '{}': expected at most {} argument(s) in '{}' subdirective, got {}",
                        stringify!($block), $max, $name, directive.args.len()
                    ).into(),
                span: $directive.span.clone(),
            });
                }
                $crate::validate_nested!(@check_args $block, directive, [$($pattern $(if $guard)?),+], $name);
            }
        }
    };

    // Single subdirective with arg range and type patterns (array syntax)
    ($block:expr, $name:ident, args($range:expr) => [$($pattern:pat $(if $guard:expr)?),+]) => {
        if let Some(directives) = $block.directives.get(stringify!($name)) {
            for directive in directives {
                if !$range.contains(&directive.args.len()) {
                    return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                        "Invalid directive '{}': expected {} argument(s) in '{}' subdirective, got {}",
                        stringify!($block), $range, $name, directive.args.len()
                    ).into(),
                span: $directive.span.clone(),
            });
                }
                $crate::validate_nested!(@check_args $block, directive, [$($pattern $(if $guard)?),+], $name);
            }
        }
    };

    // Single subdirective with any number of args and type patterns (array syntax)
    ($block:expr, $name:ident, args(?) => [$($pattern:pat $(if $guard:expr)?),+]) => {
        if let Some(directives) = $block.directives.get(stringify!($name)) {
            for directive in directives {
                $crate::validate_nested!(@check_args $block, directive, [$($pattern $(if $guard)?),+], $name);
            }
        }
    };

    // Internal helper to check argument types
    (@check_args $block:expr, $directive:ident, [$pattern:pat $(if $guard:expr)?], $subdirective_name:ident) => {
        if !$directive.args.is_empty() && !matches!($directive.args[0], $pattern $(if $guard)?) {
            return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                "Invalid directive '{}': invalid type for '{}' subdirective at position 0",
                stringify!($block), stringify!($subdirective_name)
            ).into(),
                span: $directive.span.clone(),
            });
        }
    };

    (@check_args $block:expr, $directive:ident, [$($pattern:pat $(if $guard:expr)?),+], $subdirective_name:ident) => {
        $crate::validate_nested!(@check_args_impl $block, $directive, 0, [$($pattern $(if $guard)?),+], $subdirective_name)
    };

    (@check_args_impl $block:expr, $directive:ident, $idx:expr, [$pattern:pat $(if $guard:expr)?], $subdirective_name:ident) => {
        if !$directive.args.is_empty() && !matches!($directive.args[$idx], $pattern $(if $guard)?) {
            return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                "Invalid directive '{}': invalid type for '{}' subdirective at position {}",
                stringify!($block), stringify!($subdirective_name), $idx
            ).into(),
                span: $directive.span.clone(),
            });
        }
    };

    (@check_args_impl $block:expr, $directive:ident, $idx:expr, [$pattern:pat $(if $guard:expr)?, $($rest:tt)+], $subdirective_name:ident) => {
        if !$directive.args.is_empty() && !matches!($directive.args[$idx], $pattern $(if $guard)?) {
            return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                "Invalid directive '{}': invalid type for '{}' subdirective at position {}",
                stringify!($block), stringify!($subdirective_name), $idx
            ).into(),
                span: $directive.span.clone(),
            });
        }
        $crate::validate_nested!(@check_args_impl $block, $directive, $idx + 1, [$($rest)+], $subdirective_name)
    };

    // Legacy syntax - single pattern without array (for backwards compatibility)
    ($block:expr, $name:ident, args($count:expr) => $pattern:pat $(if $guard:expr)?) => {
        if let Some(directives) = $block.directives.get(stringify!($name)) {
            for directive in directives {
                if directive.args.len() != $count {
                    return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                        "Invalid directive '{}': expected {} argument(s) in '{}' subdirective, got {}",
                        stringify!($block), $count, stringify!($name), directive.args.len()
                    ).into(),
                span: $directive.span.clone(),
            });
                }
                if !directive.args.is_empty() &&  !matches!(directive.args[0], $pattern $(if $guard)?) {
                    return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                        "Invalid directive '{}': invalid type for '{}' subdirective",
                        stringify!($block), stringify!($name)
                    ).into(),
                span: $directive.span.clone(),
            });
                }
            }
        }
    };

    ($block:expr, $name:ident, args(min = $min:expr) => $pattern:pat $(if $guard:expr)?) => {
        if let Some(directives) = $block.directives.get(stringify!($name)) {
            for directive in directives {
                if directive.args.len() < $min {
                    return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                        "Invalid directive '{}': expected at least {} argument(s) in '{}' subdirective, got {}",
                        stringify!($block), $min, $name, directive.args.len()
                    ).into(),
                span: $directive.span.clone(),
            });
                }
                if !directive.args.is_empty() &&  !matches!(directive.args[0], $pattern $(if $guard)?) {
                    return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                        "Invalid directive '{}': invalid type for '{}' subdirective",
                        stringify!($block), stringify!($name)
                    ).into(),
                span: $directive.span.clone(),
            });
                }
            }
        }
    };

    ($block:expr, $name:ident, args(max = $max:expr) => $pattern:pat $(if $guard:expr)?) => {
        if let Some(directives) = $block.directives.get(stringify!($name)) {
            for directive in directives {
                if directive.args.len() > $max {
                    return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                        "Invalid directive '{}': expected at most {} argument(s) in '{}' subdirective, got {}",
                        stringify!($block), $max, $name, directive.args.len()
                    ).into(),
                span: $directive.span.clone(),
            });
                }
                if  !directive.args.is_empty() && !matches!(directive.args[0], $pattern $(if $guard)?) {
                    return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                        "Invalid directive '{}': invalid type for '{}' subdirective",
                        stringify!($block), stringify!($name)
                    ).into(),
                span: $directive.span.clone(),
            });
                }
            }
        }
    };

    ($block:expr, $name:ident, args($range:expr) => $pattern:pat $(if $guard:expr)?) => {
        if let Some(directives) = $block.directives.get(stringify!($name)) {
            for directive in directives {
                if !$range.contains(&directive.args.len()) {
                    return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                        "Invalid directive '{}': expected {} argument(s) in '{}' subdirective, got {}",
                        stringify!($block), $range, $name, directive.args.len()
                    ).into(),
                span: $directive.span.clone(),
            });
                }
                if !directive.args.is_empty() && !matches!(directive.args[0], $pattern $(if $guard)?) {
                    return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                        "Invalid directive '{}': invalid type for '{}' subdirective",
                        stringify!($block), stringify!($name)
                    ).into(),
                span: $directive.span.clone(),
            });
                }
            }
        }
    };

    ($block:expr, $name:ident, args(?) => $pattern:pat $(if $guard:expr)?) => {
        if let Some(directives) = $block.directives.get(stringify!($name)) {
            for directive in directives {
                if !directive.args.is_empty() &&  !matches!(directive.args[0], $pattern $(if $guard)?) {
                    return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                        "Invalid directive '{}': invalid type for '{}' subdirective",
                        stringify!($block), stringify!($name)
                    ).into(),
                span: $directive.span.clone(),
            });
                }
            }
        }
    };

    // Subdirective with block (for deeper nesting)
    ($block:expr, $name:ident, $body:block) => {
        if let Some(directives) = $block.directives.get(stringify!($name)) {
            for directive in directives {
                let __empty = Default::default();
                let $name = directive.children.as_ref().unwrap_or(&__empty);
                $body
            }
        }
    };

    // Optional subdirective with no args and block
    ($block:expr, $name:ident, optional no_args, $body:block) => {
        if let Some(directives) = $block.directives.get(stringify!($name)) {
            for directive in directives {
                if !directive.args.is_empty() {
                    return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                        "Invalid directive '{}': expected no arguments in '{}' subdirective",
                        stringify!($name), stringify!($name)
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

    // Subdirective with no children validation (just check existence)
    ($block:expr, $name:ident) => {
        if let Some(directives) = $block.directives.get(stringify!($name)) {
            let _ = directives;
        }
    };

    // Optional subdirective - no error if missing, no args
    ($block:expr, $name:ident, optional) => {
        if let Some(directives) = $block.directives.get(stringify!($name)) {
            let _ = directives;
        }
    };

    // Optional subdirective with exact arg count and type patterns
    ($block:expr, $name:ident, optional args($count:expr) => [$($pattern:pat $(if $guard:expr)?),+]) => {
        if let Some(directives) = $block.directives.get(stringify!($name)) {
            for directive in directives {
                if directive.args.len() != $count {
                    return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                        "Invalid directive '{}': expected {} argument(s) in '{}' subdirective, got {}",
                        stringify!($block), $count, stringify!($name), directive.args.len()
                    ).into(),
                span: directive.span.clone(),
            });
                }
                $crate::validate_nested!(@check_args $block, directive, [$($pattern $(if $guard)?),+], $name);
            }
        }
    };

    // Optional subdirective with minimum arg count and type patterns
    ($block:expr, $name:ident, optional args(min = $min:expr) => [$($pattern:pat $(if $guard:expr)?),+]) => {
        if let Some(directives) = $block.directives.get(stringify!($name)) {
            for directive in directives {
                if directive.args.len() < $min {
                    return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                        "Invalid directive '{}': expected at least {} argument(s) in '{}' subdirective, got {}",
                        stringify!($block), $min, $name, directive.args.len()
                    ).into(),
                span: $directive.span.clone(),
            });
                }
                $crate::validate_nested!(@check_args $block, directive, [$($pattern $(if $guard)?),+], $name);
            }
        }
    };

    // Optional subdirective with maximum arg count and type patterns
    ($block:expr, $name:ident, optional args(max = $max:expr) => [$($pattern:pat $(if $guard:expr)?),+]) => {
        if let Some(directives) = $block.directives.get(stringify!($name)) {
            for directive in directives {
                if directive.args.len() > $max {
                    return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                        "Invalid directive '{}': expected at most {} argument(s) in '{}' subdirective, got {}",
                        stringify!($block), $max, $name, directive.args.len()
                    ).into(),
                span: $directive.span.clone(),
            });
                }
                $crate::validate_nested!(@check_args $block, directive, [$($pattern $(if $guard)?),+], $name);
            }
        }
    };

    // Optional subdirective with arg range and type patterns
    ($block:expr, $name:ident, optional args($range:expr) => [$($pattern:pat $(if $guard:expr)?),+]) => {
        if let Some(directives) = $block.directives.get(stringify!($name)) {
            for directive in directives {
                if !$range.contains(&directive.args.len()) {
                    return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                        "Invalid directive '{}': expected {} argument(s) in '{}' subdirective, got {}",
                        stringify!($block), $range, $name, directive.args.len()
                    ).into(),
                span: $directive.span.clone(),
            });
                }
                $crate::validate_nested!(@check_args $block, directive, [$($pattern $(if $guard)?),+], $name);
            }
        }
    };

    // Optional subdirective with any number of args and type patterns
    ($block:expr, $name:ident, optional args(?) => [$($pattern:pat $(if $guard:expr)?),+]) => {
        if let Some(directives) = $block.directives.get(stringify!($name)) {
            for directive in directives {
                $crate::validate_nested!(@check_args $block, directive, [$($pattern $(if $guard)?),+], $name);
            }
        }
    };

    // Optional subdirective with any number of arguments and type pattern validation
    ($block:expr, $name:ident, optional args(*) => [$($pattern:pat $(if $guard:expr)?),+]) => {
        if let Some(directives) = $block.directives.get(stringify!($name)) {
            for directive in directives {
                $crate::validate_nested!(@check_all_args $block, directive, 0, [$($pattern $(if $guard)?),+], $name);
            }
        }
    };

    // Optional subdirective with multiple variations
    ($block:expr, $name:ident, optional args($count1:expr) => [$($pattern1:pat),+] $(| args($countN:expr) => [$($patternN:pat),+])+) => {
        if let Some(directives) = $block.directives.get(stringify!($name)) {
            for directive in directives {
                let mut matched = false;
                if directive.args.len() == $count1 {
                    if directive.args.is_empty() || $crate::validate_nested!(@check_bool directive, [$($pattern1),+]) {
                        matched = true;
                    }
                }
                $(
                    if !matched && directive.args.len() == $countN {
                        if directive.args.is_empty() || $crate::validate_nested!(@check_bool directive, [$($patternN),+]) {
                            matched = true;
                        }
                    }
                )+
                if !matched {
                    return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                        "Invalid directive '{}': argument count or type mismatch in '{}' subdirective (expected one of the valid signatures)",
                        stringify!($block), stringify!($name)
                    ).into(),
                span: directive.span.clone(),
            });
                }
            }
        }
    };

    // Multiple variations with "or" operator - subdirective can have one of several signatures
    ($block:expr, $name:ident, args($count1:expr) => [$($pattern1:pat),+] $(| args($countN:expr) => [$($patternN:pat),+])+) => {
        if let Some(directives) = $block.directives.get(stringify!($name)) {
            for directive in directives {
                let mut matched = false;
                if directive.args.len() == $count1 {
                    if directive.args.is_empty() || $crate::validate_nested!(@check_bool directive, [$($pattern1),+]) {
                        matched = true;
                    }
                }
                $(
                    if !matched && directive.args.len() == $countN {
                        if directive.args.is_empty() || $crate::validate_nested!(@check_bool directive, [$($patternN),+]) {
                            matched = true;
                        }
                    }
                )+
                if !matched {
                    return Err($crate::config::validator::ConfigurationValidationError {
                inner: format!(
                        "Invalid directive '{}': argument count or type mismatch in '{}' subdirective (expected one of the valid signatures)",
                        stringify!($block), stringify!($name)
                    ).into(),
                span: directive.span.clone(),
            });
                }
            }
        }
    };

    // Boolean check helper - returns true if patterns match (for use in "or" variations)
    (@check_bool $directive:ident, [$($pattern:pat $(if $guard:expr)?),+]) => {
        $crate::validate_nested!(@check_bool_impl $directive, 0, [$($pattern $(if $guard)?),+])
    };

    (@check_bool_impl $directive:ident, $idx:expr, [$pattern:pat $(if $guard:expr)?]) => {
       !$directive.args.is_empty() && matches!($directive.args[$idx], $pattern $(if $guard)?)
    };

    (@check_bool_impl $directive:ident, $idx:expr, [$pattern:pat $(if $guard:expr)?, $($rest:tt)+]) => {
        !$directive.args.is_empty() && matches!($directive.args[$idx], $pattern $(if $guard)?) &&
        $crate::validate_nested!(@check_bool_impl $directive, $idx + 1, [$($rest)+])
    };
}
