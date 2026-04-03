//! Configuration validation framework.
//!
//! Validators check configuration blocks for correctness, tracking used directives
//! and reporting errors for invalid or missing configuration.

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
    /// * `used_directives` - Set of directive names that have been processed.
    ///   Validators should add recognized directives to this set.
    /// * `is_global` - Whether this is the global configuration block
    ///   (as opposed to protocol-specific or host-specific blocks)
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration is invalid (missing required directives,
    /// unknown directives, value format errors, etc.)
    fn validate_block(
        &self,
        config: &crate::config::ServerConfigurationBlock,
        used_directives: &mut std::collections::HashSet<String>,
        is_global: bool,
    ) -> Result<(), Box<dyn std::error::Error>>;
}
