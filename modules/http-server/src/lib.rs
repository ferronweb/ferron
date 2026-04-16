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
pub mod tls_auto;
#[cfg(any(test, feature = "bench"))]
pub mod util;
#[cfg(not(any(test, feature = "bench")))]
mod util;
mod validator;

pub use loader::BasicHttpModuleLoader;

#[cfg(any(test, feature = "bench"))]
pub use handler::bench_resolve_http_file_target;
