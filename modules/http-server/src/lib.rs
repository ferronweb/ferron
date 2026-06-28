//! HTTP server module for Ferron.
//!
//! Provides the core HTTP serving functionality including:
//! - TCP/TLS listener management
//! - Request handling with pipeline execution
//! - Configuration resolution via three-stage resolver
//! - Pipeline stages: ClientIpFromHeaderStage, HttpsRedirectStage

#[cfg(any(test, feature = "fuzz"))]
pub mod config;
#[cfg(not(any(test, feature = "fuzz")))]
mod config;

#[cfg(feature = "fuzz")]
pub mod handler;
#[cfg(not(feature = "fuzz"))]
mod handler;
mod loader;
mod server;
#[cfg(any(test, feature = "fuzz"))]
pub mod stages;
#[cfg(not(any(test, feature = "fuzz")))]
mod stages;
pub mod tls_auto;
#[cfg(any(test, feature = "fuzz"))]
pub mod util;
#[cfg(not(any(test, feature = "fuzz")))]
mod util;
mod validator;

pub use loader::BasicHttpModuleLoader;
