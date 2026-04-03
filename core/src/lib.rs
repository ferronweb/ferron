//! Ferron core library providing the foundation for module-based server architecture.
//!
//! This library provides:
//! - Module trait for pluggable server components
//! - Configuration system with validation and adaptation
//! - Registry for stages and providers with DAG-based ordering
//! - Pipeline execution with inverse operations
//! - Logging with Windows Event Log and stdio backends
//! - Runtime management with primary and secondary task execution

#[macro_use]
pub mod config;
pub mod builtin;
pub mod loader;
pub mod logging;
pub mod pipeline;
pub mod providers;
pub mod registry;
pub mod runtime;
pub mod shutdown;

pub use registry::StageConstraint;

use std::any::Any;

/// Trait for pluggable server modules in the Ferron architecture.
///
/// Modules are server implementations (HTTP, TCP, etc.) that can be registered
/// at runtime and started with access to the shared registry and runtime.
///
/// # Examples
///
/// ```ignore
/// struct MyHttpModule;
///
/// impl Module for MyHttpModule {
///     fn name(&self) -> &str {
///         "http"
///     }
///
///     fn as_any(&self) -> &dyn Any {
///         self
///     }
///
///     fn start(
///         &self,
///         runtime: &mut Runtime,
///     ) -> Result<(), Box<dyn std::error::Error>> {
///         // Initialize and start the module
///         Ok(())
///     }
/// }
/// ```
pub trait Module: Send + Sync {
    /// Returns the name of this module.
    fn name(&self) -> &str;

    /// Returns this trait object as `Any` for downcasting to concrete type.
    fn as_any(&self) -> &dyn Any;

    /// Start the module with access to the runtime and registry.
    ///
    /// This is called during server startup to initialize the module.
    fn start(
        &self,
        runtime: &mut crate::runtime::Runtime,
    ) -> Result<(), Box<dyn std::error::Error>>;
}
