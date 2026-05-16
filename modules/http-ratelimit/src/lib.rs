//! HTTP rate limiting module for Ferron.
//!
//! Provides the `rate_limit` directive for request rate control using token bucket
//! algorithms with configurable keys (IP, URI, custom headers).

mod config;
mod key_extractor;
mod loader;
#[cfg(any(test, feature = "bench", feature = "fuzz"))]
pub mod registry;
#[cfg(not(any(test, feature = "bench", feature = "fuzz")))]
mod registry;
mod stage;
#[cfg(any(test, feature = "bench", feature = "fuzz"))]
pub mod token_bucket;
#[cfg(not(any(test, feature = "bench", feature = "fuzz")))]
mod token_bucket;
mod validator;

pub use loader::HttpRateLimitModuleLoader;
