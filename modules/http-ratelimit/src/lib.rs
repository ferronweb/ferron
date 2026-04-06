//! HTTP rate limiting module for Ferron.
//!
//! Provides the `rate_limit` directive for request rate control using token bucket
//! algorithms with configurable keys (IP, URI, custom headers).

mod config;
mod key_extractor;
mod loader;
mod registry;
mod stage;
mod token_bucket;
mod validator;

pub use loader::HttpRateLimitModuleLoader;
