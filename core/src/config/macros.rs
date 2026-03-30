/// Validate a top-level directive in a configuration block.
///
/// # Usage
///
/// ```ignore
/// // Directive with no arguments
/// validate_directive!(config, used, "runtime", no_args, {
///     // body runs for each directive instance
///     // `directive` variable is bound to ServerConfigurationDirectiveEntry
///     // `runtime` variable is bound to the children block (&ServerConfigurationBlock)
/// });
///
/// // Directive with exact argument count
/// validate_directive!(config, used, "port", args(1), {
///     // validates arg count, body runs for each instance
/// });
///
/// // Directive with argument type patterns
/// validate_directive!(config, used, "port", args(1) => [ServerConfigurationValue::Number(_, _)], {
///     // validates arg count and types
/// });
///
/// // Optional directive (no error if missing)
/// validate_directive!(config, used, "debug", optional, {
///     // only runs if directive exists
/// });
/// ```
#[macro_export]
macro_rules! validate_directive {
    // No arguments expected
    ($config:expr, $used:expr, $name:ident, no_args, $body:block) => {
        $crate::validate_directive!(@inner $config, $used, $name, 0, $body)
    };

    // Exact argument count
    ($config:expr, $used:expr, $name:ident, args($count:expr), $body:block) => {
        $crate::validate_directive!(@inner $config, $used, $name, $count, $body)
    };

    // Exact argument count with type patterns
    ($config:expr, $used:expr, $name:ident, args($count:expr) => [$($pattern:pat),+], $body:block) => {
        $crate::validate_directive!(@inner $config, $used, $name, $count, {
            $crate::validate_args!(directive, [$($pattern),+]);
            $body
        })
    };

    // Optional directive (no error if missing)
    ($config:expr, $used:expr, $name:ident, optional, $body:block) => {
        if let Some(directives) = $config.directives.get(stringify!($name)) {
            $used.insert(stringify!($name).to_string());
            for directive in directives {
                let $name = directive.children.as_ref();
                $body
            }
        }
    };

    // Internal implementation
    (@inner $config:expr, $used:expr, $name:ident, $count:expr, $body:block) => {
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
    ($directive:expr, [$pattern:pat]) => {
        if !matches!($directive.args[0], $pattern) {
            return Err("Invalid directive: argument type mismatch at position 0".into());
        }
    };

    ($directive:expr, [$($pattern:pat),+]) => {
        $(
            if !matches!($directive.args[$crate::validate_args!(@idx $pattern)], $pattern) {
                return Err(format!(
                    "Invalid directive: argument type mismatch at position {}",
                    $crate::validate_args!(@idx $pattern)
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
/// // Single subdirective with arg count and type pattern
/// validate_nested!(block, "io_uring", args(1) => ServerConfigurationValue::Boolean(_, _));
///
/// // Subdirective with nested block for deeper nesting
/// validate_nested!(block, "pool", {
///     validate_nested!(pool, "size", args(1) => ServerConfigurationValue::Number(_, _));
/// });
/// ```
#[macro_export]
macro_rules! validate_nested {
    // Single subdirective with arg count and type pattern
    ($block:expr, $name:literal, args($count:expr) => $pattern:pat) => {
        if let Some(directives) = $block.directives.get($name) {
            for directive in directives {
                if directive.args.len() != $count {
                    return Err(format!(
                        "Invalid directive '{}': expected {} argument(s) in '{}' subdirective, got {}",
                        stringify!($block), $count, $name, directive.args.len()
                    ).into());
                }
                if !matches!(directive.args[0], $pattern) {
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
