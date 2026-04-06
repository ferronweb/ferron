//! HTTP server module for Ferron.
//!
//! Provides the core HTTP serving functionality including:
//! - TCP/TLS listener management
//! - Request handling with pipeline execution
//! - Configuration resolution via three-stage resolver
//! - Pipeline stages: ClientIpFromHeaderStage, HttpsRedirectStage

#[cfg(any(test, feature = "bench"))]
pub mod config;
#[cfg(not(any(test, feature = "bench")))]
mod config;

mod handler;
mod loader;
mod server;
mod stages;
mod util;
mod validator;

pub use loader::BasicHttpModuleLoader;
