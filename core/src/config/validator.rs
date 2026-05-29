//! Configuration validation framework.
//!
//! Validators check configuration blocks for correctness, tracking used directives
//! and reporting errors for invalid or missing configuration.

/// A key for scoped configuration validators.
#[derive(Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConfigurationValidatorScopedKey {
    pub namespace: &'static str,
    pub module: String,
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
}
