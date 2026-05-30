//! Configuration validation framework.
//!
//! Validators check configuration blocks for correctness, tracking used directives
//! and reporting errors for invalid or missing configuration.

use serde::Serialize;

use crate::config::ServerConfigurationSpan;

/// A key for scoped configuration validators.
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
/// Validators are called during configuration loading to check that:
/// - All directives are recognized
/// - Required directives are present
/// - Values are in the correct format
/// - Dependencies between directives are satisfied
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
    ) -> Result<(), Box<dyn std::error::Error>>;
}

/// Context for tracking used directives and other data during configuration validation.
pub struct ConfigurationValidatorContext {
    /// Set of directive names that have been processed by validators.
    pub used_directives: std::collections::HashSet<String>,
    /// Whether this is the global configuration block (as opposed to protocol-specific
    /// or host-specific blocks).
    pub is_global: bool,
    /// Validators scoped to specific namespaces and modules.
    pub scoped_validators: std::sync::Arc<
        std::collections::HashMap<
            ConfigurationValidatorScopedKey,
            Box<dyn crate::config::validator::ConfigurationValidator>,
        >,
    >,
    /// Collected configuration diagnostics.
    pub diagnostics: Vec<crate::config::validator::ConfigurationValidatorDiagnostic>,
    /// Configuration scope (for example, block)
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
) -> Result<(), Box<dyn std::error::Error>> {
    let mut local_ctx = ConfigurationValidatorContext {
        used_directives: std::collections::HashSet::new(),
        is_global: false, // Inapplicable for scoped configurations
        scoped_validators: ctx.scoped_validators.clone(), // Allow sub-scopes
        diagnostics: Vec::new(),
        scope: ctx.scope.clone(),
    };

    // Validate provider and get scoped validator
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
    for unused_directive in block
        .directives
        .keys()
        .filter(|dn| !local_ctx.used_directives.contains(*dn))
    {
        ctx.diagnostics.push(local_ctx.create_diagnostic(
            ConfigurationValidatorDiagnosticKind::UnknownDirective,
            format!(
                "`{unused_directive}` is unused in the block for `{provider_namespace}` namespace"
            ),
            block.span.clone(),
        ));
    }
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

/// Represents a diagnostic from a [`ConfigurationValidator`].
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

/// The diagnostic kinds for configuration validation diagnostics.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum ConfigurationValidatorDiagnosticKind {
    /// The configuration block contains an invalid or unknown directive.
    InvalidConfiguration,
    /// A directive was encountered that is not recognized by the validator.
    UnknownDirective,
    /// A best practice violation was detected in the configuration.
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
