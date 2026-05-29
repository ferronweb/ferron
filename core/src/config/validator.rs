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

    provider_validator.validate_block(block, &mut local_ctx)
}
