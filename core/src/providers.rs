//! Provider trait for pluggable implementations of domain-specific functionality.
//!
//! Providers are the core abstraction for pluggable services in Ferron.
//! Each provider implements [`Provider<C>`] for a specific context type `C`,
//! and is registered in the [`Registry`](crate::registry::Registry) by name
//! and type. Modules discover providers at runtime without compile-time
//! dependencies.
//!
//! # Common provider types
//!
//! | Context type | Purpose |
//! |---|---|
//! | TLS context | Certificate resolution, OCSP stapling |
//! | DNS context | Domain name resolution |
//! | Observability context | Log and metrics sinks |
//!
//! # Writing a provider
//!
//! 1. Define a context type `C` that carries the input/output for your
//!    provider.
//! 2. Implement `Provider<C>` on a struct.
//! 3. Register it in your [`ModuleLoader`](crate::loader::ModuleLoader) via
//!    [`RegistryBuilder::with_provider`](crate::registry::RegistryBuilder::with_provider).
//! 4. Retrieve it at runtime via
//!    [`ProviderRegistry::get`](crate::registry::ProviderRegistry::get).

/// A pluggable component for domain-specific functionality.
///
/// Providers are registered by name and context type in the
/// [`Registry`](crate::registry::Registry). Other modules discover them at
/// runtime via [`ProviderRegistry::get`](crate::registry::ProviderRegistry::get)
/// without needing a compile-time dependency on the provider's crate.
///
/// # Lifecycle
///
/// 1. The provider is registered in
///    [`ModuleLoader::register_providers`](crate::loader::ModuleLoader::register_providers).
/// 2. At runtime, a consumer calls
///    [`Registry::get_provider_registry`](crate::registry::Registry::get_provider_registry)
///    to get the typed [`ProviderRegistry`](crate::registry::ProviderRegistry).
/// 3. The consumer calls [`ProviderRegistry::get`](crate::registry::ProviderRegistry::get)
///    with the provider's name to create an instance via the factory.
/// 4. The consumer calls [`execute`](Self::execute) with a mutable context.
///
/// # Example
///
/// ```ignore
/// use ferron_core::providers::Provider;
///
/// struct MyCacheProvider;
///
/// impl Provider<CacheContext> for MyCacheProvider {
///     fn name(&self) -> &str {
///         "memory"
///     }
///
///     fn execute(
///         &self,
///         ctx: &mut CacheContext,
///     ) -> Result<(), Box<dyn std::error::Error>> {
///         // ... perform caching operation ...
///         Ok(())
///     }
/// }
/// ```
pub trait Provider<C>: Send + Sync {
    /// Returns the unique name of this provider.
    ///
    /// The name is used for lookup in [`ProviderRegistry::get`](crate::registry::ProviderRegistry::get)
    /// and corresponds to the `provider` directive value in configuration.
    fn name(&self) -> &str;

    /// Execute the provider with the given context.
    ///
    /// The context type `C` is application-specific (e.g. a TLS context with
    /// a domain name, or an observability context with an event). The
    /// provider reads from and writes to the context as needed.
    ///
    /// Return `Ok(())` on success or `Err` to propagate the error to the
    /// caller.
    fn execute(&self, ctx: &mut C) -> Result<(), Box<dyn std::error::Error>>;
}
