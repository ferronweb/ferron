//! Module loader trait for runtime registration of modules and components.
//!
//! The [`ModuleLoader`] trait defines the extension points through which
//! modules register configuration adapters, validators, stages, providers,
//! and directives. All methods have default no-op implementations, so a
//! module only overrides the methods it needs.
//!
//! # Registration order
//!
//! The server calls [`ModuleLoader`] methods in this order during
//! initialization:
//!
//! 1. [`register_configuration_adapters`](ModuleLoader::register_configuration_adapters)
//! 2. [`register_per_protocol_configuration_blocks`](ModuleLoader::register_per_protocol_configuration_blocks)
//! 3. [`register_global_configuration_validators`](ModuleLoader::register_global_configuration_validators)
//! 4. [`register_per_protocol_configuration_validators`](ModuleLoader::register_per_protocol_configuration_validators)
//! 5. [`register_scoped_configuration_validators`](ModuleLoader::register_scoped_configuration_validators)
//! 6. [`register_stages`](ModuleLoader::register_stages)
//! 7. [`register_providers`](ModuleLoader::register_providers)
//! 8. [`register_directives`](ModuleLoader::register_directives)
//! 9. [`register_modules`](ModuleLoader::register_modules)
//!
//! # Example
//!
//! ```ignore
//! use ferron_core::loader::ModuleLoader;
//! use ferron_core::registry::RegistryBuilder;
//!
//! #[derive(Default)]
//! struct MyModuleLoader;
//!
//! impl ModuleLoader for MyModuleLoader {
//!     fn register_stages(
//!         &mut self,
//!         registry: RegistryBuilder,
//!     ) -> RegistryBuilder {
//!         registry.with_stage::<MyContext, _>(|| Arc::new(MyStage))
//!     }
//! }

use std::collections::HashMap;
use std::sync::Arc;

use crate::config::adapter::ConfigurationAdapter;
use crate::config::validator::ConfigurationValidatorScopedKey;
use crate::directives::DirectiveRegistry;

/// Extension point for registering a module's components and configuration.
///
/// Implement this trait on a `#[derive(Default)]` struct and override only
/// the methods your module needs. The server calls each method exactly once
/// during initialization (see [module-level docs](self) for the call order).
///
/// # Architecture
///
/// Modules do not depend on each other at compile time. Instead they
/// communicate through the [`Registry`](crate::registry::Registry): stages are executed in
/// topologically sorted order, and providers are discovered by type and name
/// at runtime. This means any module can use functionality exported by
/// another module without importing its crate.
///
/// # Example
///
/// ```ignore
/// use std::sync::Arc;
/// use ferron_core::loader::ModuleLoader;
/// use ferron_core::registry::RegistryBuilder;
///
/// #[derive(Default)]
/// struct CacheModuleLoader;
///
/// impl ModuleLoader for CacheModuleLoader {
///     fn register_stages(
///         &mut self,
///         registry: RegistryBuilder,
///     ) -> RegistryBuilder {
///         registry.with_stage::<MyContext, _>(|| Arc::new(CacheStage))
///     }
///
///     fn register_providers(
///         &mut self,
///         registry: RegistryBuilder,
///     ) -> RegistryBuilder {
///         registry.with_provider::<MyProvider, _>(|| Arc::new(MemoryCache))
///     }
/// }
/// ```
pub trait ModuleLoader {
    /// Register configuration blocks for specific protocols.
    ///
    /// Maps protocol names (e.g. `"http"`, `"https"`) to their
    /// corresponding [`ServerConfigurationBlock`](crate::config::ServerConfigurationBlock)
    /// entries. The server calls this method once per registered module.
    ///
    /// Most modules do not need to override this method. It is primarily
    /// used by protocol-level modules that own a wire format.
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
    /// Validators registered here run against the top-level (global)
    /// configuration block. Use this to enforce rules that apply
    /// regardless of protocol or host.
    #[allow(unused_variables)]
    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn crate::config::validator::ConfigurationValidator>>,
    ) {
    }

    /// Register protocol-specific validation rules.
    ///
    /// Validators registered here run against configuration blocks scoped
    /// to a specific protocol. The `HashMap` keys are protocol names
    /// (e.g. `"http"`).
    #[allow(unused_variables)]
    fn register_per_protocol_configuration_validators(
        &mut self,
        registry: &mut HashMap<
            &'static str,
            Vec<Box<dyn crate::config::validator::ConfigurationValidator>>,
        >,
    ) {
    }

    /// Register scoped configuration validators for nested blocks.
    ///
    /// Scoped validators are keyed by
    /// [`ConfigurationValidatorScopedKey`],
    /// which combines a namespace (e.g. `"tls"`, `"observability"`) with a
    /// provider name (e.g. `"local"`, `"cloudflare"`). They are invoked
    /// when a configuration block selects a specific provider via a
    /// `provider` directive.
    #[allow(unused_variables)]
    fn register_scoped_configuration_validators(
        &mut self,
        registry: &mut HashMap<
            ConfigurationValidatorScopedKey,
            Box<dyn crate::config::validator::ConfigurationValidator>,
        >,
    ) {
    }

    /// Register configuration adapters.
    ///
    /// Adapters load [`ServerConfiguration`](crate::config::ServerConfiguration)
    /// from a source (file, database, etc.) and produce a
    /// [`ConfigurationWatcher`](crate::config::adapter::ConfigurationWatcher)
    /// for change detection. The `HashMap` keys are adapter names (e.g.
    /// `"ferronconf"`).
    #[allow(unused_variables)]
    fn register_configuration_adapters(
        &mut self,
        registry: &mut HashMap<&'static str, Box<dyn ConfigurationAdapter>>,
    ) {
    }

    /// Register pipeline stages with optional ordering constraints.
    ///
    /// Stages are [`Stage`](crate::pipeline::Stage) implementations that execute in
    /// topologically sorted order. Use
    /// [`RegistryBuilder::with_stage`](crate::registry::RegistryBuilder::with_stage)
    /// to register them. The generic type parameter (`C`) is the context
    /// type the stage operates on (e.g. `HttpContext`).
    ///
    /// The server calls this method once per module. Return the builder
    /// unchanged if your module does not register stages.
    fn register_stages(
        &mut self,
        registry: crate::registry::RegistryBuilder,
    ) -> crate::registry::RegistryBuilder {
        registry
    }

    /// Register typed providers for domain-specific functionality.
    ///
    /// Providers are [`Provider`](crate::providers::Provider) implementations discovered by
    /// type and name at runtime. Use
    /// [`RegistryBuilder::with_provider`](crate::registry::RegistryBuilder::with_provider)
    /// to register them. The generic type parameter (`C`) is the context
    /// type the provider operates on.
    ///
    /// Return the builder unchanged if your module does not register
    /// providers.
    #[allow(unused_variables)]
    fn register_providers(
        &mut self,
        registry: crate::registry::RegistryBuilder,
    ) -> crate::registry::RegistryBuilder {
        registry
    }

    /// Register module implementations and initialize resources.
    ///
    /// This is the final initialization hook. By the time it is called, all
    /// configuration adapters, validators, stages, providers, and directives
    /// have been registered. Use this method to:
    ///
    /// - Read finalized configuration from the [`ServerConfiguration`](crate::config::ServerConfiguration).
    /// - Set up global state (e.g. event sinks, caches).
    /// - Create [`Module`](crate::Module) instances and push them into `modules`.
    ///
    /// Return `Err` to abort server startup with an error message.
    #[allow(unused_variables)]
    fn register_modules(
        &mut self,
        registry: Arc<crate::registry::Registry>,
        modules: &mut Vec<Arc<dyn crate::Module>>,
        config: Arc<crate::config::ServerConfiguration>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    /// Register directives for the `ferron directives` subcommand.
    ///
    /// Directives are metadata that the CLI prints as JSON for editor
    /// support (autocomplete, validation). See the
    /// [`directives`](crate::directives) module for the registration API.
    #[allow(unused_variables)]
    fn register_directives(&mut self, registry: &mut DirectiveRegistry) {}
}
