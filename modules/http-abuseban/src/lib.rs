//! Lightweight abuse protection with temporary IP banning.
//!
//! This module provides Fail2ban-like protection: tracks abusive behavior
//! (rate limit breaches, brute-force failures) and temporarily bans IPs
//! that exceed configured thresholds within time windows.
//!
//! Other HTTP modules can report abuse events to trigger IP bans via the
//! global abuse recorder (defined in `ferron_http::abuse`). Rate limiting and
//! basic auth modules emit events through the shared trait -- they do not
//! depend on this crate directly.

mod config;
mod loader;
mod registry;
mod stage;
mod validator;

pub use loader::HttpAbuseProtectionModuleLoader;
