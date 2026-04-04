//! Module loader trait for runtime registration of modules and components.
//!
//! The `ModuleLoader` trait defines the extension points through which modules
//! register stages, providers, validators, and configuration adapters.

use std::{collections::HashMap, sync::Arc};

use crate::config::adapter::ConfigurationAdapter;

/// Trait for modules to register their components and configuration.
///
/// This trait provides extension points for modules to:
/// - Define protocol-specific configuration
/// - Register validation rules
/// - Register configuration adapters
/// - Register stages with ordering constraints
/// - Register providers
/// - Register module implementations
///
/// All methods have default no-op implementations, so modules only override
/// the extension points they need.
pub trait ModuleLoader {
    /// Register configuration blocks for specific protocols.
    ///
    /// Allows modules to define which configuration sections apply to their protocol.
    /// Called once per module during initialization.
    #[allow(unused_variables)]
    fn register_per_protocol_configuration_blocks<'a>(
        &mut self,
        config: &'a crate::config::ServerConfiguration,
        registry: &mut HashMap<
            &'static str,
            Vec<(String, &'a crate::config::ServerConfigurationBlock)>,
        >,
    ) {
    }

    /// Register global validation rules for configuration.
    ///
    /// Called once per module to register validators for the global config section.
    #[allow(unused_variables)]
    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn crate::config::validator::ConfigurationValidator>>,
    ) {
    }

    /// Register protocol-specific validation rules.
    ///
    /// Called once per module to register validators for protocol-specific sections.
    #[allow(unused_variables)]
    fn register_per_protocol_configuration_validators(
        &mut self,
        registry: &mut HashMap<
            &'static str,
            Box<dyn crate::config::validator::ConfigurationValidator>,
        >,
    ) {
    }

    /// Register configuration adapters.
    ///
    /// Configuration adapters transform raw configuration values into typed structures.
    #[allow(unused_variables)]
    fn register_configuration_adapters(
        &mut self,
        registry: &mut HashMap<&'static str, Box<dyn ConfigurationAdapter>>,
    ) {
    }

    /// Register pipeline stages with optional ordering constraints.
    ///
    /// Stages are used to build ordered pipelines for request processing.
    fn register_stages(
        &mut self,
        registry: crate::registry::RegistryBuilder,
    ) -> crate::registry::RegistryBuilder {
        registry
    }

    /// Register typed providers for domain-specific functionality.
    ///
    /// Providers are discovered by type and name, allowing modules to
    /// extend functionality without compile-time dependencies.
    #[allow(unused_variables)]
    fn register_providers(
        &mut self,
        registry: crate::registry::RegistryBuilder,
    ) -> crate::registry::RegistryBuilder {
        registry
    }

    /// Register module implementations and initialize resources.
    ///
    /// This is called after all configuration and stages have been registered.
    /// Modules can now access the full registry and create their server implementations.
    #[allow(unused_variables)]
    fn register_modules(
        &mut self,
        registry: Arc<crate::registry::Registry>,
        modules: &mut Vec<Arc<dyn crate::Module>>,
        config: Arc<crate::config::ServerConfiguration>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }
}
