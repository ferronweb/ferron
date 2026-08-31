//! Core library for the Ferron web server.
//!
//! This crate defines the extension points that module authors use to plug
//! into the server. It is protocol-agnostic: it knows nothing about HTTP, TLS,
//! DNS, or any specific wire format.
//!
//! # Key concepts
//!
//! | Concept | Purpose |
//! |---|---|
//! | [`Module`] / [`ModuleLoader`](crate::loader::ModuleLoader) | Registration and lifecycle hooks for server components |
//! | [`pipeline::Stage`] | Ordered processing steps (e.g. request/response pipeline) |
//! | [`providers::Provider`] | Pluggable domain-specific implementations (e.g. TLS, DNS, cache) |
//! | [`registry::Registry`] | Type-erased container that holds typed stage and provider registries |
//! | [`config`] | Hierarchical configuration with validation, adapters, and layered overrides |
//! | [`runtime::Runtime`] | Dual-runtime model: primary (zincio, per-CPU) and secondary (tokio) |
//! | [`logging`] | Lightweight logging backend (stdio on all platforms, Event Log on Windows) |
//! | [`admin`] | Global atomic metrics shared between the data plane and admin API |
//! | [`shutdown`] | Application-wide cancellation tokens for shutdown and reload |
//!
//! # Module lifecycle
//!
//! 1. [`ModuleLoader`](crate::loader::ModuleLoader) methods are called during initialization to register
//!    configuration adapters, validators, stages, providers, and directives.
//! 2. [`Module::start`] is called for each registered module, giving it access
//!    to the [`runtime::Runtime`] for spawning tasks.
//! 3. At runtime, [`pipeline::Stage`] instances execute in topologically sorted
//!    order, with [`run_inverse`](pipeline::Stage::run_inverse) called in
//!    reverse order on completion.
//!
//! # Writing an external module
//!
//! Implement [`ModuleLoader`](crate::loader::ModuleLoader) on a `#[derive(Default)]` struct, override only
//! the methods you need, and register your module in the server's entrypoint.
//! See the [`loader`] module for details.

#[macro_use]
pub mod config;
pub mod admin;
pub mod builtin;
pub mod directives;
pub mod loader;
pub mod logging;
pub mod pipeline;
pub mod providers;
pub mod registry;
pub mod runtime;
pub mod shutdown;

use std::any::Any;

/// A server component that can be registered and started at runtime.
///
/// Modules are the primary extension point in Ferron. Each module provides
/// a [`ModuleLoader`](crate::loader::ModuleLoader) (during initialization) and a `Module` instance (at
/// runtime). The module's [`start`](Self::start) method receives a mutable
/// reference to the [`runtime::Runtime`], which it uses to spawn primary or
/// secondary tasks.
///
/// Modules do not interact with each other directly. Instead, they
/// communicate through the shared [`registry::Registry`] (typed stages and
/// providers) and the application-wide [`shutdown`] and [`admin`] state.
///
/// # Example
///
/// ```ignore
/// struct MyModule;
///
/// impl Module for MyModule {
///     fn name(&self) -> &str {
///         "my_module"
///     }
///
///     fn as_any(&self) -> &dyn std::any::Any {
///         self
///     }
///
///     fn start(
///         &self,
///         runtime: &mut ferron_core::runtime::Runtime,
///     ) -> Result<(), Box<dyn std::error::Error>> {
///         runtime.spawn_secondary_task(async {
///             // background work
///         });
///         Ok(())
///     }
/// }
/// ```
pub trait Module: Send + Sync {
    /// Returns the unique name of this module.
    ///
    /// The name is used for identification and logging. It must be stable
    /// across server restarts because configuration may reference it.
    fn name(&self) -> &str;

    /// Returns this trait object as [`Any`] for downcasting.
    ///
    /// The default implementation returns `self`, which works for most cases.
    fn as_any(&self) -> &dyn Any;

    /// Start the module after all configuration and stages have been registered.
    ///
    /// Use [`runtime::Runtime::spawn_primary_task`] for per-CPU I/O-intensive
    /// work (listeners, connection loops) and
    /// [`runtime::Runtime::spawn_secondary_task`] for background tasks that
    /// do not need dedicated CPU threads (metrics aggregation, certificate
    /// renewal, etc.).
    ///
    /// Return `Ok(())` on success. A non-zero exit from `start` is considered
    /// a fatal initialization error.
    fn start(
        &self,
        runtime: &mut crate::runtime::Runtime,
    ) -> Result<(), Box<dyn std::error::Error>>;
}
