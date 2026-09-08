//! HTTP rate limiting module for Ferron.
//!
//! Provides the `rate_limit` directive for request rate control using token bucket
//! algorithms with configurable keys (IP, URI, custom headers).

#[cfg(any(test, feature = "fuzz"))]
pub mod backends;
#[cfg(not(any(test, feature = "fuzz")))]
mod backends;
mod config;
mod key_extractor;
mod loader;
mod stage;
mod validator;

pub use loader::{HttpRateLimitModule, HttpRateLimitModuleLoader};
