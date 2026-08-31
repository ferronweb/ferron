//! Configuration validation framework.
//!
//! Validators check configuration blocks for correctness during loading.
//! They track which directives have been used and report errors (and
//! optional best-practice diagnostics) for invalid or missing configuration.
//!
//! # Writing a validator
//!
//! 1. Implement [`ConfigurationValidator`] on a struct.
//! 2. Register it in your [`ModuleLoader`](crate::loader::ModuleLoader) via
//!    [`register_global_configuration_validators`](crate::loader::ModuleLoader::register_global_configuration_validators),
//!    [`register_per_protocol_configuration_validators`](crate::loader::ModuleLoader::register_per_protocol_configuration_validators),
//!    or [`register_scoped_configuration_validators`](crate::loader::ModuleLoader::register_scoped_configuration_validators).
//! 3. Use the [`validate_directive!`] and [`validate_nested!`] macros to
//!    check directive structure and argument types.
//!
//! # Diagnostic levels
//!
//! | Kind | Meaning |
//! |---|---|
//! | [`InvalidConfiguration`](ConfigurationValidatorDiagnosticKind::InvalidConfiguration) | Hard error: the config is invalid and the server cannot start |
//! | [`UnknownDirective`](ConfigurationValidatorDiagnosticKind::UnknownDirective) | A directive was not recognized by any validator |
//! | [`BestPracticeViolation`](ConfigurationValidatorDiagnosticKind::BestPracticeViolation) | Soft warning: the config works but may cause issues |

use serde::Serialize;

use crate::config::{
    ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationSpan,
    ServerConfigurationValue,
};

/// A key for scoped configuration validators.
///
/// Scoped validators are registered for a specific namespace (e.g. `"tls"`,
/// `"observability"`) and provider name (e.g. `"local"`, `"cloudflare"`).
/// They are invoked when a configuration block selects a provider via a
/// `provider` directive within that namespace.
///
/// Use the [`config_validator_scoped_key!`] macro to create instances.
#[derive(Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConfigurationValidatorScopedKey {
    pub namespace: &'static str,
    pub module: String,
}

/// Macro for creating [`ConfigurationValidatorScopedKey`] values.
#[macro_export]
macro_rules! config_validator_scoped_key {
    ($namespace:literal, $module:expr) => {
        (::ferron_core::config::validator::ConfigurationValidatorScopedKey {
            namespace: $namespace,
            module: $module.to_string(),
        })
    };
}

/// Validator for configuration blocks.
///
/// Implement this trait on a struct and register it in your
/// [`ModuleLoader`](crate::loader::ModuleLoader). The server calls
/// [`validate_block`](Self::validate_block) once per configuration block
/// during loading.
///
/// Use the [`validate_directive!`] and [`validate_nested!`] macros to check
/// directive structure and argument types. Track processed directives via
/// [`ConfigurationValidatorContext::used_directives`] and emit
/// diagnostics via [`ConfigurationValidatorContext::add_best_practice_violation`].
///
/// # Example
///
/// ```ignore
/// struct MyValidator;
///
/// impl ConfigurationValidator for MyValidator {
///     fn validate_block(
///         &self,
///         config: &ServerConfigurationBlock,
///         ctx: &mut ConfigurationValidatorContext,
///     ) -> Result<(), ConfigurationValidationError> {
///         let used = &mut ctx.used_directives;
///         validate_directive!(config, used, my_directive, optional args(1) => [
///             ServerConfigurationValue::String(_, _)
///         ], {
///             // directive is valid
///         });
///         Ok(())
///     }
/// }
/// ```
pub trait ConfigurationValidator {
    /// Validate a configuration block.
    ///
    /// # Arguments
    ///
    /// * `config` - The configuration block to validate
    /// * `validator_ctx` - Context for tracking used directives and other data.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration is invalid (missing required directives,
    /// unknown directives, value format errors, etc.)
    fn validate_block(
        &self,
        config: &crate::config::ServerConfigurationBlock,
        validator_ctx: &mut ConfigurationValidatorContext,
    ) -> Result<(), ConfigurationValidationError>;
}

/// Context passed to [`ConfigurationValidator::validate_block`].
///
/// Tracks which directives have been processed (via [`used_directives`])
/// and collects diagnostics. After all validators run, any directive not
/// in `used_directives` is reported as
/// [`UnknownDirective`](ConfigurationValidatorDiagnosticKind::UnknownDirective).
///
/// [`used_directives`]: ConfigurationValidatorContext::used_directives
pub struct ConfigurationValidatorContext {
    /// Set of directive names that have been processed by validators.
    ///
    /// Any directive not in this set after validation triggers an
    /// [`UnknownDirective`](ConfigurationValidatorDiagnosticKind::UnknownDirective)
    /// diagnostic.
    pub used_directives: std::collections::HashSet<String>,
    /// Whether this is the global configuration block (as opposed to
    /// protocol-specific or host-specific blocks).
    pub is_global: bool,
    /// Scoped validators registered by modules, keyed by
    /// [`ConfigurationValidatorScopedKey`].
    pub scoped_validators: std::sync::Arc<
        std::collections::HashMap<
            ConfigurationValidatorScopedKey,
            Box<dyn crate::config::validator::ConfigurationValidator>,
        >,
    >,
    /// Collected configuration diagnostics (errors and best-practice
    /// violations).
    pub diagnostics: Vec<crate::config::validator::ConfigurationValidatorDiagnostic>,
    /// Optional scope label for diagnostics (e.g. a host block name).
    pub scope: Option<String>,
}

impl ConfigurationValidatorContext {
    pub fn create_diagnostic(
        &self,
        kind: ConfigurationValidatorDiagnosticKind,
        message: impl Into<String>,
        span: Option<crate::config::ServerConfigurationSpan>,
    ) -> ConfigurationValidatorDiagnostic {
        ConfigurationValidatorDiagnostic {
            kind,
            message: message.into(),
            span,
            scope: self.scope.clone(),
        }
    }

    pub fn add_best_practice_violation(
        &mut self,
        message: impl Into<String>,
        span: Option<crate::config::ServerConfigurationSpan>,
    ) {
        let diagnostic = self.create_diagnostic(
            ConfigurationValidatorDiagnosticKind::BestPracticeViolation,
            message,
            span,
        );
        self.diagnostics.push(diagnostic);
    }
}

/// Validates a block of configuration that is scoped to a specific namespace/module.
pub fn validate_scoped_block(
    block: &super::ServerConfigurationBlock,
    ctx: &mut ConfigurationValidatorContext,
    provider_field: &'static str,
    provider_namespace: &'static str,
    default_provider: Option<&str>,
) -> Result<(), ConfigurationValidationError> {
    let mut local_ctx = ConfigurationValidatorContext {
        used_directives: std::collections::HashSet::new(),
        is_global: false, // Inapplicable for scoped configurations
        scoped_validators: ctx.scoped_validators.clone(), // Allow sub-scopes
        diagnostics: Vec::new(),
        scope: ctx.scope.clone(),
    };

    let Some(provider) = block
        .get_value(provider_field)
        .and_then(|s| s.as_str())
        .or(default_provider)
    else {
        Err(anyhow::anyhow!(
            "Missing or invalid provider name for `{}`",
            provider_namespace
        ))?
    };
    local_ctx.used_directives.insert(provider_field.to_string());
    let Some(provider_validator) = ctx.scoped_validators.get(&ConfigurationValidatorScopedKey {
        namespace: provider_namespace,
        module: provider.to_string(),
    }) else {
        Err(anyhow::anyhow!(
            "`{}` provider not found: {}",
            provider_namespace,
            provider
        ))?
    };

    let result = provider_validator.validate_block(block, &mut local_ctx);
    ctx.diagnostics.append(&mut local_ctx.diagnostics);
    for (unused_directive, span) in block
        .directives
        .iter()
        .filter(|d| !local_ctx.used_directives.contains(d.0))
        .flat_map(|d| d.1.iter().map(|s| (d.0.clone(), s.span.clone())))
    {
        ctx.diagnostics.push(local_ctx.create_diagnostic(
            ConfigurationValidatorDiagnosticKind::UnknownDirective,
            format!(
                "`{unused_directive}` is unused in the block for `{provider_namespace}` namespace"
            ),
            span.or(block.span.clone()),
        ));
    }
    result
}

/// Validates a block of configuration that is scoped to a specific namespace/module,
/// in a "flat" style where the provider directive is expected to be at the same level as
/// other configuration directives, rather than in a nested block.
pub fn validate_scoped_block_flat(
    block: &super::ServerConfigurationBlock,
    ctx: &mut ConfigurationValidatorContext,
    provider_field: &'static str,
    provider_namespace: &'static str,
    default_provider: Option<&str>,
) -> Result<(), ConfigurationValidationError> {
    let mut local_ctx = ConfigurationValidatorContext {
        used_directives: std::collections::HashSet::new(),
        is_global: false, // Inapplicable for scoped configurations
        scoped_validators: ctx.scoped_validators.clone(), // Allow sub-scopes
        diagnostics: Vec::new(),
        scope: ctx.scope.clone(),
    };

    let Some(provider) = block
        .get_value(provider_field)
        .and_then(|s| s.as_str())
        .or(default_provider)
    else {
        Err(anyhow::anyhow!(
            "Missing or invalid provider name for `{}`",
            provider_namespace
        ))?
    };
    local_ctx.used_directives.insert(provider_field.to_string());
    let Some(provider_validator) = ctx.scoped_validators.get(&ConfigurationValidatorScopedKey {
        namespace: provider_namespace,
        module: provider.to_string(),
    }) else {
        Err(anyhow::anyhow!(
            "`{}` provider not found: {}",
            provider_namespace,
            provider
        ))?
    };

    let result = provider_validator.validate_block(block, &mut local_ctx);
    ctx.used_directives.extend(local_ctx.used_directives);
    ctx.diagnostics.extend(local_ctx.diagnostics);
    result
}

fn format_location(block_name: Option<&str>, span: Option<&ServerConfigurationSpan>) -> String {
    let mut location = String::new();
    if let Some(name) = block_name {
        location.push_str(&format!("block '{}'", name));
    } else {
        location.push_str("global configuration");
    }
    if let Some(span) = span {
        if let Some(file) = &span.file {
            location.push_str(&format!(" in file '{}'", file));
        }
        location.push_str(&format!(" at line {}", span.line));
        location.push_str(&format!(", column {}", span.column));
    }
    location
}

/// A diagnostic emitted by a [`ConfigurationValidator`].
///
/// Diagnostics carry a severity kind, a human-readable message, and an
/// optional source location span. They are collected during validation and
/// reported to the user (or exposed via the admin API as JSON).
#[derive(Clone, Debug, Serialize)]
pub struct ConfigurationValidatorDiagnostic {
    /// The kind of the diagnostic.
    pub kind: ConfigurationValidatorDiagnosticKind,
    /// A human-readable message describing the diagnostic.
    pub message: String,
    /// The span in the configuration file where this diagnostic occurred.
    pub span: Option<ServerConfigurationSpan>,
    /// Optional scope for the diagnostic (e.g., a specific block).
    pub scope: Option<String>,
}

impl std::fmt::Display for ConfigurationValidatorDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} ({}): {}",
            self.kind,
            format_location(self.scope.as_deref(), self.span.as_ref()),
            self.message
        )
    }
}

/// The severity kind of a [`ConfigurationValidatorDiagnostic`].
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum ConfigurationValidatorDiagnosticKind {
    /// The configuration block contains an invalid or unknown directive.
    /// This is a hard error that prevents the server from starting.
    InvalidConfiguration,
    /// A directive was encountered that is not recognized by any registered
    /// validator. This is typically a typo or an unrecognized module directive.
    UnknownDirective,
    /// A best practice violation was detected. This is a soft warning that
    /// does not prevent the server from starting.
    BestPracticeViolation,
}

impl std::fmt::Display for ConfigurationValidatorDiagnosticKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind_str = match self {
            Self::InvalidConfiguration => "Invalid configuration",
            Self::UnknownDirective => "Unknown directive",
            Self::BestPracticeViolation => "Best practice violation",
        };
        write!(f, "{kind_str}")?;
        Ok(())
    }
}

impl Serialize for ConfigurationValidatorDiagnosticKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_str(self)
    }
}

/// An error that occurred while validating configuration.
///
/// Carries the underlying error and an optional source location span for
/// pointing back to the offending directive.
#[derive(Debug)]
pub struct ConfigurationValidationError {
    /// The underlying error that occurred.
    pub inner: Box<dyn std::error::Error>,
    /// The span of the configuration that caused the error, if known.
    pub span: Option<ServerConfigurationSpan>,
}

impl std::error::Error for ConfigurationValidationError {}

impl std::fmt::Display for ConfigurationValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&*self.inner, f)
    }
}

impl From<anyhow::Error> for ConfigurationValidationError {
    fn from(inner: anyhow::Error) -> Self {
        Self {
            inner: inner.into_boxed_dyn_error(),
            span: None,
        }
    }
}

impl From<Box<dyn std::error::Error>> for ConfigurationValidationError {
    fn from(inner: Box<dyn std::error::Error>) -> Self {
        Self { inner, span: None }
    }
}

impl From<std::io::Error> for ConfigurationValidationError {
    fn from(inner: std::io::Error) -> Self {
        Self {
            inner: Box::new(inner),
            span: None,
        }
    }
}

impl From<String> for ConfigurationValidationError {
    fn from(inner: String) -> Self {
        Self {
            inner: anyhow::anyhow!(inner).into_boxed_dyn_error(),
            span: None,
        }
    }
}

impl From<&'static str> for ConfigurationValidationError {
    fn from(inner: &'static str) -> Self {
        Self {
            inner: anyhow::anyhow!(inner).into_boxed_dyn_error(),
            span: None,
        }
    }
}

impl ConfigurationValidationError {
    pub fn with_span(mut self, span: Option<ServerConfigurationSpan>) -> Self {
        self.span = span;
        self
    }
}

/// Extract the source location span from a directive entry.
///
/// Returns the entry's own span, falling back to the first argument's span
/// if the entry has no span. Useful for attaching location info to
/// diagnostics.
pub fn entry_span(entry: &ServerConfigurationDirectiveEntry) -> Option<ServerConfigurationSpan> {
    entry.span.clone().or_else(|| {
        entry.args.first().and_then(|value| match value {
            ServerConfigurationValue::String(_, span)
            | ServerConfigurationValue::Number(_, span)
            | ServerConfigurationValue::Float(_, span)
            | ServerConfigurationValue::Boolean(_, span)
            | ServerConfigurationValue::InterpolatedString(_, span) => span.clone(),
        })
    })
}

/// Extract the source location span for the first entry of a named directive.
///
/// Returns the span of the first entry for the given directive in the
/// block, or `None` if the directive does not exist.
pub fn first_entry_span(
    block: &ServerConfigurationBlock,
    directive: &str,
) -> Option<ServerConfigurationSpan> {
    block
        .directives
        .get(directive)
        .and_then(|entries| entries.first())
        .and_then(entry_span)
}
