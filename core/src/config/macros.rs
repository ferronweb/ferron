//! Macros for configuration validation and building.
//!
//! Provides convenience macros for:
//! - Validating configuration directives with pattern matching
//! - Building configuration structures with a fluent API

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
                        return Err(format!(
                            "Invalid directive '{}': argument type mismatch at position {}",
                            stringify!($name), idx
                        ).into());
                    }
                }
                let __empty = Default::default();
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
                        return Err(format!(
                            "Invalid directive '{}': argument type mismatch at position {}",
                            stringify!($name), idx
                        ).into());
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
                // Check first variation
                if directive.args.len() == $count1 {
                    if $crate::validate_args!(@check directive, [$($pattern1),+]) {
                        matched = true;
                    }
                }
                // Check remaining variations
                $(
                    if !matched && directive.args.len() == $countN {
                        if $crate::validate_args!(@check directive, [$($patternN),+]) {
                            matched = true;
                        }
                    }
                )+
                if !matched {
                    return Err(format!(
                        "Invalid directive '{}': argument count or type mismatch (expected one of the valid signatures)",
                        stringify!($name)
                    ).into());
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
                // Check first variation
                if directive.args.len() == $count1 {
                    if $crate::validate_args!(@check directive, [$($pattern1),+]) {
                        matched = true;
                    }
                }
                // Check remaining variations
                $(
                    if !matched && directive.args.len() == $countN {
                        if $crate::validate_args!(@check directive, [$($patternN),+]) {
                            matched = true;
                        }
                    }
                )+
                if !matched && !directive.args.is_empty() {
                    return Err(format!(
                        "Invalid directive '{}': argument count or type mismatch (expected one of the valid signatures)",
                        stringify!($name)
                    ).into());
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
                    return Err(format!(
                        "Invalid directive '{}': expected {} argument(s), got {}",
                        stringify!($name), $count, directive.args.len()
                    ).into());
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
                    return Err(format!(
                        "Invalid directive '{}': expected {} argument(s), got {}",
                        stringify!($name), $count, directive.args.len()
                    ).into());
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
                    return Err(format!(
                        "Invalid directive '{}': expected at least {} argument(s), got {}",
                        stringify!($name), $min, directive.args.len()
                    ).into());
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
                    return Err(format!(
                        "Invalid directive '{}': expected at least {} argument(s), got {}",
                        stringify!($name), $min, directive.args.len()
                    ).into());
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
                    return Err(format!(
                        "Invalid directive '{}': expected at most {} argument(s), got {}",
                        stringify!($name), $max, directive.args.len()
                    ).into());
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
                    return Err(format!(
                        "Invalid directive '{}': expected at most {} argument(s), got {}",
                        stringify!($name), $max, directive.args.len()
                    ).into());
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
                    return Err(format!(
                        "Invalid directive '{}': expected {}-{} argument(s), got {}",
                        stringify!($name), $range.min().unwrap_or(0), $range.max().unwrap_or(0), directive.args.len()
                    ).into());
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
                    return Err(format!(
                        "Invalid directive '{}': expected {}-{} argument(s), got {}",
                        stringify!($name), $range.min().unwrap_or(0), $range.max().unwrap_or(0), directive.args.len()
                    ).into());
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
                    return Err(format!(
                        "Invalid directive '{}': expected {} argument(s), got {}",
                        stringify!($name), $count, directive.args.len()
                    ).into());
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
                        return Err(format!(
                            "Invalid directive '{}': expected {} argument(s), got {}",
                            stringify!($name), $count, directive.args.len()
                        ).into());
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
                    return Err(format!(
                        "Invalid directive '{}': expected at least {} argument(s), got {}",
                        stringify!($name), $min, directive.args.len()
                    ).into());
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
                        return Err(format!(
                            "Invalid directive '{}': expected at least {} argument(s), got {}",
                            stringify!($name), $min, directive.args.len()
                        ).into());
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
                    return Err(format!(
                        "Invalid directive '{}': expected at most {} argument(s), got {}",
                        stringify!($name), $max, directive.args.len()
                    ).into());
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
                        return Err(format!(
                            "Invalid directive '{}': expected at most {} argument(s), got {}",
                            stringify!($name), $max, directive.args.len()
                        ).into());
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
                    return Err(format!(
                        "Invalid directive '{}': expected {} argument(s), got {}",
                        stringify!($name), $range, directive.args.len()
                    ).into());
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
                        return Err(format!(
                            "Invalid directive '{}': expected {} argument(s), got {}",
                            stringify!($name), $range, directive.args.len()
                        ).into());
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
        if !matches!($directive.args[0], $pattern $(if $guard)?) {
            return Err("Invalid directive: argument type mismatch at position 0".into());
        }
    };

    // Multiple patterns - use internal helper with counter
    ($directive:expr, [$($pattern:pat $(if $guard:expr)?),+]) => {
        $crate::validate_args!(@multi $directive, 0, [$($pattern $(if $guard)?),+])
    };

    // Internal: process multiple patterns with index counter
    (@multi $directive:expr, $idx:expr, [$pattern:pat $(if $guard:expr)?]) => {
        if !matches!($directive.args[$idx], $pattern $(if $guard)?) {
            return Err(format!(
                "Invalid directive: argument type mismatch at position {}",
                $idx
            ).into());
        }
    };

    (@multi $directive:expr, $idx:expr, [$pattern:pat $(if $guard:expr)?, $($rest:tt)+]) => {
        if !matches!($directive.args[$idx], $pattern $(if $guard)?) {
            return Err(format!(
                "Invalid directive: argument type mismatch at position {}",
                $idx
            ).into());
        }
        $crate::validate_args!(@multi $directive, $idx + 1, [$($rest)+])
    };

    // Check helper - returns true if patterns match, false otherwise (for use in "or" variations)
    (@check $directive:expr, [$($pattern:pat $(if $guard:expr)?),+]) => {
        $crate::validate_args!(@check_impl $directive, 0, [$($pattern $(if $guard)?),+])
    };

    (@check_impl $directive:expr, $idx:expr, [$pattern:pat $(if $guard:expr)?]) => {
        matches!($directive.args[$idx], $pattern $(if $guard)?)
    };

    (@check_impl $directive:expr, $idx:expr, [$pattern:pat $(if $guard:expr)?, $($rest:tt)+]) => {
        matches!($directive.args[$idx], $pattern $(if $guard)?) &&
        $crate::validate_args!(@check_impl $directive, $idx + 1, [$($rest)+])
    };
}

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
    // Any number of arguments with type pattern validation (array syntax) - must come before args($count:expr)
    ($block:expr, $name:ident, args(*) => [$($pattern:pat $(if $guard:expr)?),+]) => {
        if let Some(directives) = $block.directives.get(stringify!($name)) {
            for directive in directives {
                for (idx, arg) in directive.args.iter().enumerate() {
                    if !matches!(arg, $($pattern $(if $guard)?)+) {
                        return Err(format!(
                            "Invalid directive '{}': invalid type for '{}' subdirective at position {}",
                            stringify!($block), stringify!($name), idx
                        ).into());
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
                    return Err(format!(
                        "Invalid directive '{}': expected {} argument(s) in '{}' subdirective, got {}",
                        stringify!($block), $count, stringify!($name), directive.args.len()
                    ).into());
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
                    return Err(format!(
                        "Invalid directive '{}': expected at least {} argument(s) in '{}' subdirective, got {}",
                        stringify!($block), $min, $name, directive.args.len()
                    ).into());
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
                    return Err(format!(
                        "Invalid directive '{}': expected at most {} argument(s) in '{}' subdirective, got {}",
                        stringify!($block), $max, $name, directive.args.len()
                    ).into());
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
                    return Err(format!(
                        "Invalid directive '{}': expected {} argument(s) in '{}' subdirective, got {}",
                        stringify!($block), $range, $name, directive.args.len()
                    ).into());
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
        if !matches!($directive.args[0], $pattern $(if $guard)?) {
            return Err(format!(
                "Invalid directive '{}': invalid type for '{}' subdirective at position 0",
                stringify!($block), stringify!($subdirective_name)
            ).into());
        }
    };

    (@check_args $block:expr, $directive:ident, [$($pattern:pat $(if $guard:expr)?),+], $subdirective_name:ident) => {
        $crate::validate_nested!(@check_args_impl $block, $directive, 0, [$($pattern $(if $guard)?),+], $subdirective_name)
    };

    (@check_args_impl $block:expr, $directive:ident, $idx:expr, [$pattern:pat $(if $guard:expr)?], $subdirective_name:ident) => {
        if !matches!($directive.args[$idx], $pattern $(if $guard)?) {
            return Err(format!(
                "Invalid directive '{}': invalid type for '{}' subdirective at position {}",
                stringify!($block), stringify!($subdirective_name), $idx
            ).into());
        }
    };

    (@check_args_impl $block:expr, $directive:ident, $idx:expr, [$pattern:pat $(if $guard:expr)?, $($rest:tt)+], $subdirective_name:ident) => {
        if !matches!($directive.args[$idx], $pattern $(if $guard)?) {
            return Err(format!(
                "Invalid directive '{}': invalid type for '{}' subdirective at position {}",
                stringify!($block), stringify!($subdirective_name), $idx
            ).into());
        }
        $crate::validate_nested!(@check_args_impl $block, $directive, $idx + 1, [$($rest)+], $subdirective_name)
    };

    // Legacy syntax - single pattern without array (for backwards compatibility)
    ($block:expr, $name:ident, args($count:expr) => $pattern:pat $(if $guard:expr)?) => {
        if let Some(directives) = $block.directives.get(stringify!($name)) {
            for directive in directives {
                if directive.args.len() != $count {
                    return Err(format!(
                        "Invalid directive '{}': expected {} argument(s) in '{}' subdirective, got {}",
                        stringify!($block), $count, stringify!($name), directive.args.len()
                    ).into());
                }
                if !matches!(directive.args[0], $pattern $(if $guard)?) {
                    return Err(format!(
                        "Invalid directive '{}': invalid type for '{}' subdirective",
                        stringify!($block), stringify!($name)
                    ).into());
                }
            }
        }
    };

    ($block:expr, $name:ident, args(min = $min:expr) => $pattern:pat $(if $guard:expr)?) => {
        if let Some(directives) = $block.directives.get(stringify!($name)) {
            for directive in directives {
                if directive.args.len() < $min {
                    return Err(format!(
                        "Invalid directive '{}': expected at least {} argument(s) in '{}' subdirective, got {}",
                        stringify!($block), $min, $name, directive.args.len()
                    ).into());
                }
                if !matches!(directive.args[0], $pattern $(if $guard)?) {
                    return Err(format!(
                        "Invalid directive '{}': invalid type for '{}' subdirective",
                        stringify!($block), stringify!($name)
                    ).into());
                }
            }
        }
    };

    ($block:expr, $name:ident, args(max = $max:expr) => $pattern:pat $(if $guard:expr)?) => {
        if let Some(directives) = $block.directives.get(stringify!($name)) {
            for directive in directives {
                if directive.args.len() > $max {
                    return Err(format!(
                        "Invalid directive '{}': expected at most {} argument(s) in '{}' subdirective, got {}",
                        stringify!($block), $max, $name, directive.args.len()
                    ).into());
                }
                if !matches!(directive.args[0], $pattern $(if $guard)?) {
                    return Err(format!(
                        "Invalid directive '{}': invalid type for '{}' subdirective",
                        stringify!($block), stringify!($name)
                    ).into());
                }
            }
        }
    };

    ($block:expr, $name:ident, args($range:expr) => $pattern:pat $(if $guard:expr)?) => {
        if let Some(directives) = $block.directives.get(stringify!($name)) {
            for directive in directives {
                if !$range.contains(&directive.args.len()) {
                    return Err(format!(
                        "Invalid directive '{}': expected {} argument(s) in '{}' subdirective, got {}",
                        stringify!($block), $range, $name, directive.args.len()
                    ).into());
                }
                if !matches!(directive.args[0], $pattern $(if $guard)?) {
                    return Err(format!(
                        "Invalid directive '{}': invalid type for '{}' subdirective",
                        stringify!($block), stringify!($name)
                    ).into());
                }
            }
        }
    };

    ($block:expr, $name:ident, args(?) => $pattern:pat $(if $guard:expr)?) => {
        if let Some(directives) = $block.directives.get(stringify!($name)) {
            for directive in directives {
                if !matches!(directive.args[0], $pattern $(if $guard)?) {
                    return Err(format!(
                        "Invalid directive '{}': invalid type for '{}' subdirective",
                        stringify!($block), stringify!($name)
                    ).into());
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
                    return Err(format!(
                        "Invalid directive '{}': expected {} argument(s) in '{}' subdirective, got {}",
                        stringify!($block), $count, stringify!($name), directive.args.len()
                    ).into());
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
                    return Err(format!(
                        "Invalid directive '{}': expected at least {} argument(s) in '{}' subdirective, got {}",
                        stringify!($block), $min, $name, directive.args.len()
                    ).into());
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
                    return Err(format!(
                        "Invalid directive '{}': expected at most {} argument(s) in '{}' subdirective, got {}",
                        stringify!($block), $max, $name, directive.args.len()
                    ).into());
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
                    return Err(format!(
                        "Invalid directive '{}': expected {} argument(s) in '{}' subdirective, got {}",
                        stringify!($block), $range, $name, directive.args.len()
                    ).into());
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
                // Check first variation
                if directive.args.len() == $count1 {
                    if $crate::validate_nested!(@check_bool directive, [$($pattern1),+]) {
                        matched = true;
                    }
                }
                // Check remaining variations
                $(
                    if !matched && directive.args.len() == $countN {
                        if $crate::validate_nested!(@check_bool directive, [$($patternN),+]) {
                            matched = true;
                        }
                    }
                )+
                if !matched {
                    return Err(format!(
                        "Invalid directive '{}': argument count or type mismatch in '{}' subdirective (expected one of the valid signatures)",
                        stringify!($block), stringify!($name)
                    ).into());
                }
            }
        }
    };

    // Multiple variations with "or" operator - subdirective can have one of several signatures
    ($block:expr, $name:ident, args($count1:expr) => [$($pattern1:pat),+] $(| args($countN:expr) => [$($patternN:pat),+])+) => {
        if let Some(directives) = $block.directives.get(stringify!($name)) {
            for directive in directives {
                let mut matched = false;
                // Check first variation
                if directive.args.len() == $count1 {
                    if $crate::validate_nested!(@check_bool directive, [$($pattern1),+]) {
                        matched = true;
                    }
                }
                // Check remaining variations
                $(
                    if !matched && directive.args.len() == $countN {
                        if $crate::validate_nested!(@check_bool directive, [$($patternN),+]) {
                            matched = true;
                        }
                    }
                )+
                if !matched {
                    return Err(format!(
                        "Invalid directive '{}': argument count or type mismatch in '{}' subdirective (expected one of the valid signatures)",
                        stringify!($block), stringify!($name)
                    ).into());
                }
            }
        }
    };

    // Boolean check helper - returns true if patterns match (for use in "or" variations)
    (@check_bool $directive:ident, [$($pattern:pat $(if $guard:expr)?),+]) => {
        $crate::validate_nested!(@check_bool_impl $directive, 0, [$($pattern $(if $guard)?),+])
    };

    (@check_bool_impl $directive:ident, $idx:expr, [$pattern:pat $(if $guard:expr)?]) => {
        matches!($directive.args[$idx], $pattern $(if $guard)?)
    };

    (@check_bool_impl $directive:ident, $idx:expr, [$pattern:pat $(if $guard:expr)?, $($rest:tt)+]) => {
        matches!($directive.args[$idx], $pattern $(if $guard)?) &&
        $crate::validate_nested!(@check_bool_impl $directive, $idx + 1, [$($rest)+])
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
