//! Provider trait for pluggable implementations of domain-specific functionality.
//!
//! Providers are typed components (e.g., DNS providers, cache providers) that can
//! be registered in the registry and executed with domain-specific context.

/// A provider is a pluggable component for domain-specific functionality.
///
/// Providers are registered by name and category, allowing modules to discover
/// and use implementations without hard-coding dependencies.
///
/// # Examples
///
/// ```ignore
/// struct CloudflareDnsProvider;
///
/// impl Provider<DnsContext> for CloudflareDnsProvider {
///     fn name(&self) -> &str {
///         "cloudflare"
///     }
///
///     fn execute(&self, ctx: &mut DnsContext) -> Result<(), Box<dyn std::error::Error>> {
///         // Perform DNS resolution
///         Ok(())
///     }
/// }
/// ```
pub trait Provider<C>: Send + Sync {
    /// Returns the name of this provider.
    fn name(&self) -> &str;

    /// Execute the provider with the given context.
    ///
    /// The context type is application-specific (e.g., `DnsContext`, `CacheContext`).
    fn execute(&self, ctx: &mut C) -> Result<(), Box<dyn std::error::Error>>;
}
