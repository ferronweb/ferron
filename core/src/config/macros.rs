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
/// // Optional directive (no error if missing)
/// validate_directive!(config, used, debug, optional, {
///     // only runs if directive exists
/// });
///
/// // Optional directive with argument validation
/// validate_directive!(config, used, timeout, optional args(1) => [ServerConfigurationValue::Number(_, _)], {
///     // only runs if directive exists and has correct arg type
/// });
/// ```
#[macro_export]
macro_rules! validate_directive {
    // No arguments expected
    ($config:expr, $used:expr, $name:ident, no_args, $body:block) => {
        $crate::validate_directive!(@inner $config, $used, $name, exact 0, $body)
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
                let $name = directive.children.as_ref()
                    .ok_or(format!("Invalid directive '{}': missing nested block", stringify!($name)))?;
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
                let $name = directive.children.as_ref()
                    .ok_or(format!("Invalid directive '{}': missing nested block", stringify!($name)))?;
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
                let $name = directive.children.as_ref()
                    .ok_or(format!("Invalid directive '{}': missing nested block", stringify!($name)))?;
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
                let $name = directive.children.as_ref()
                    .ok_or(format!("Invalid directive '{}': missing nested block", stringify!($name)))?;
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
                let $name = directive.children.as_ref()
                    .ok_or(format!("Invalid directive '{}': missing nested block", stringify!($name)))?;
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
                let $name = directive.children.as_ref()
                    .ok_or(format!("Invalid directive '{}': missing nested block", stringify!($name)))?;
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
                        "Invalid directive '{}': expected {} argument(s), got {}",
                        stringify!($name), $range, directive.args.len()
                    ).into());
                }
                let $name = directive.children.as_ref()
                    .ok_or(format!("Invalid directive '{}': missing nested block", stringify!($name)))?;
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
                        "Invalid directive '{}': expected {} argument(s), got {}",
                        stringify!($name), $range, directive.args.len()
                    ).into());
                }
                $crate::validate_args!(directive, [$($pattern),+]);
                let $name = directive.children.as_ref()
                    .ok_or(format!("Invalid directive '{}': missing nested block", stringify!($name)))?;
                $body
            }
        }
    };

    // Internal implementation - any number of args
    (@inner $config:expr, $used:expr, $name:ident, any, $body:block) => {
        if let Some(directives) = $config.directives.get(stringify!($name)) {
            $used.insert(stringify!($name).to_string());
            for directive in directives {
                let $name = directive.children.as_ref()
                    .ok_or(format!("Invalid directive '{}': missing nested block", stringify!($name)))?;
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
                if directive.args.len() != $count {
                    return Err(format!(
                        "Invalid directive '{}': expected {} argument(s), got {}",
                        stringify!($name), $count, directive.args.len()
                    ).into());
                }
                let $name = directive.children.as_ref()
                    .ok_or(format!("Invalid directive '{}': missing nested block", stringify!($name)))?;
                $body
            }
        }
    };

    // Internal implementation for optional directives - exact count with patterns
    (@inner_optional $config:expr, $used:expr, $name:ident, exact $count:expr, [$($pattern:pat),+], $body:block) => {
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
                let $name = directive.children.as_ref()
                    .ok_or(format!("Invalid directive '{}': missing nested block", stringify!($name)))?;
                $body
            }
        }
    };

    // Internal implementation for optional directives - min without patterns
    (@inner_optional $config:expr, $used:expr, $name:ident, min $min:expr, $body:block) => {
        if let Some(directives) = $config.directives.get(stringify!($name)) {
            $used.insert(stringify!($name).to_string());
            for directive in directives {
                if directive.args.len() < $min {
                    return Err(format!(
                        "Invalid directive '{}': expected at least {} argument(s), got {}",
                        stringify!($name), $min, directive.args.len()
                    ).into());
                }
                let $name = directive.children.as_ref()
                    .ok_or(format!("Invalid directive '{}': missing nested block", stringify!($name)))?;
                $body
            }
        }
    };

    // Internal implementation for optional directives - min with patterns
    (@inner_optional $config:expr, $used:expr, $name:ident, min $min:expr, [$($pattern:pat),+], $body:block) => {
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
                let $name = directive.children.as_ref()
                    .ok_or(format!("Invalid directive '{}': missing nested block", stringify!($name)))?;
                $body
            }
        }
    };

    // Internal implementation for optional directives - max without patterns
    (@inner_optional $config:expr, $used:expr, $name:ident, max $max:expr, $body:block) => {
        if let Some(directives) = $config.directives.get(stringify!($name)) {
            $used.insert(stringify!($name).to_string());
            for directive in directives {
                if directive.args.len() > $max {
                    return Err(format!(
                        "Invalid directive '{}': expected at most {} argument(s), got {}",
                        stringify!($name), $max, directive.args.len()
                    ).into());
                }
                let $name = directive.children.as_ref()
                    .ok_or(format!("Invalid directive '{}': missing nested block", stringify!($name)))?;
                $body
            }
        }
    };

    // Internal implementation for optional directives - max with patterns
    (@inner_optional $config:expr, $used:expr, $name:ident, max $max:expr, [$($pattern:pat),+], $body:block) => {
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
                let $name = directive.children.as_ref()
                    .ok_or(format!("Invalid directive '{}': missing nested block", stringify!($name)))?;
                $body
            }
        }
    };

    // Internal implementation for optional directives - range without patterns
    (@inner_optional_range $config:expr, $used:expr, $name:ident, $range:expr, $body:block) => {
        if let Some(directives) = $config.directives.get(stringify!($name)) {
            $used.insert(stringify!($name).to_string());
            for directive in directives {
                if !$range.contains(&directive.args.len()) {
                    return Err(format!(
                        "Invalid directive '{}': expected {} argument(s), got {}",
                        stringify!($name), $range, directive.args.len()
                    ).into());
                }
                let $name = directive.children.as_ref()
                    .ok_or(format!("Invalid directive '{}': missing nested block", stringify!($name)))?;
                $body
            }
        }
    };

    // Internal implementation for optional directives - range with patterns
    (@inner_optional_range $config:expr, $used:expr, $name:ident, $range:expr, [$($pattern:pat),+], $body:block) => {
        if let Some(directives) = $config.directives.get(stringify!($name)) {
            $used.insert(stringify!($name).to_string());
            for directive in directives {
                if !$range.contains(&directive.args.len()) {
                    return Err(format!(
                        "Invalid directive '{}': expected {} argument(s), got {}",
                        stringify!($name), $range, directive.args.len()
                    ).into());
                }
                $crate::validate_args!(directive, [$($pattern),+]);
                let $name = directive.children.as_ref()
                    .ok_or(format!("Invalid directive '{}': missing nested block", stringify!($name)))?;
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
}

/// Validate argument types within a directive.
///
/// # Usage
///
/// ```ignore
/// validate_args!(directive, [ServerConfigurationValue::String(_, _)]);
/// validate_args!(directive, [
///     ServerConfigurationValue::String(_, _),
///     ServerConfigurationValue::Number(_, _)
/// ]);
/// ```
#[macro_export]
macro_rules! validate_args {
    ($directive:expr, [$pattern:pat $(if $guard:expr)?]) => {
        if !matches!($directive.args[0], $pattern $(if $guard)?) {
            return Err("Invalid directive: argument type mismatch at position 0".into());
        }
    };

    ($directive:expr, [$($pattern:pat $(if $guard:expr)?),+]) => {
        $(
            if !matches!($directive.args[$crate::validate_args!(@idx $pattern $(if $guard)?)], $pattern $(if $guard)?) {
                return Err(format!(
                    "Invalid directive: argument type mismatch at position {}",
                    $crate::validate_args!(@idx $pattern $(if $guard)?)
                ).into());
            }
        )+
    };

    (@idx $p:pat) => {
        0
    };
}

/// Validate nested subdirectives within a configuration block.
///
/// # Usage
///
/// ```ignore
/// // Single subdirective with exact arg count and type pattern
/// validate_nested!(block, io_uring, args(1) => ServerConfigurationValue::Boolean(_, _));
///
/// // Subdirective with minimum arg count and type pattern
/// validate_nested!(block, options, args(min = 1) => ServerConfigurationValue::String(_, _));
///
/// // Subdirective with maximum arg count and type pattern
/// validate_nested!(block, flags, args(max = 3) => ServerConfigurationValue::Boolean(_, _));
///
/// // Subdirective with arg range and type pattern
/// validate_nested!(block, range, args(min = 1, max = 4) => ServerConfigurationValue::Number(_, _));
///
/// // Subdirective with any number of args and type pattern
/// validate_nested!(block, items, args(?) => ServerConfigurationValue::String(_, _));
///
/// // Subdirective with nested block for deeper nesting
/// validate_nested!(block, pool, {
///     validate_nested!(pool, size, args(1) => ServerConfigurationValue::Number(_, _));
/// });
///
/// // Subdirective with just existence check
/// validate_nested!(block, debug);
/// ```
#[macro_export]
macro_rules! validate_nested {
    // Single subdirective with exact arg count and type pattern
    ($block:expr, $name:literal, args($count:expr) => $pattern:pat $(if $guard:expr)?) => {
        if let Some(directives) = $block.directives.get($name) {
            for directive in directives {
                if directive.args.len() != $count {
                    return Err(format!(
                        "Invalid directive '{}': expected {} argument(s) in '{}' subdirective, got {}",
                        stringify!($block), $count, $name, directive.args.len()
                    ).into());
                }
                if !matches!(directive.args[0], $pattern $(if $guard)?) {
                    return Err(format!(
                        "Invalid directive '{}': invalid type for '{}' subdirective",
                        stringify!($block), $name
                    ).into());
                }
            }
        }
    };

    // Single subdirective with minimum arg count and type pattern
    ($block:expr, $name:literal, args(min = $min:expr) => $pattern:pat $(if $guard:expr)?) => {
        if let Some(directives) = $block.directives.get($name) {
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
                        stringify!($block), $name
                    ).into());
                }
            }
        }
    };

    // Single subdirective with maximum arg count and type pattern
    ($block:expr, $name:literal, args(max = $max:expr) => $pattern:pat $(if $guard:expr)?) => {
        if let Some(directives) = $block.directives.get($name) {
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
                        stringify!($block), $name
                    ).into());
                }
            }
        }
    };

    // Single subdirective with arg range and type pattern
    ($block:expr, $name:literal, args($range:expr) => $pattern:pat $(if $guard:expr)?) => {
        if let Some(directives) = $block.directives.get($name) {
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
                        stringify!($block), $name
                    ).into());
                }
            }
        }
    };

    // Single subdirective with any number of args and type pattern
    ($block:expr, $name:literal, args(?) => $pattern:pat $(if $guard:expr)?) => {
        if let Some(directives) = $block.directives.get($name) {
            for directive in directives {
                if !matches!(directive.args[0], $pattern $(if $guard)?) {
                    return Err(format!(
                        "Invalid directive '{}': invalid type for '{}' subdirective",
                        stringify!($block), $name
                    ).into());
                }
            }
        }
    };

    // Subdirective with block (for deeper nesting)
    ($block:expr, $name:literal, $body:block) => {
        if let Some(directives) = $block.directives.get($name) {
            for directive in directives {
                let $name = directive.children.as_ref()
                    .ok_or(format!("Invalid directive '{}': missing nested block", $name))?;
                $body
            }
        }
    };

    // Subdirective with no children validation (just check existence)
    ($block:expr, $name:literal) => {
        if let Some(directives) = $block.directives.get($name) {
            let _ = directives;
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
        let directives = $config.directives.get($name).ok_or($error)?;
        directives
    };
}
