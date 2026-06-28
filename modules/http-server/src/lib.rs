//! HTTP server module for Ferron.
//!
//! Provides the core HTTP serving functionality including:
//! - TCP/TLS listener management
//! - Request handling with pipeline execution
//! - Configuration resolution via three-stage resolver
//! - Pipeline stages: ClientIpFromHeaderStage, HttpsRedirectStage

#[cfg(test)]
pub mod config;
#[cfg(not(test))]
mod config;

mod handler;
mod loader;
mod server;
mod stages;
pub mod tls_auto;
#[cfg(test)]
pub mod util;
#[cfg(not(test))]
mod util;
mod validator;

pub use loader::BasicHttpModuleLoader;
