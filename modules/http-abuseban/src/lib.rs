//! Lightweight abuse protection with temporary IP banning.
//!
//! This module provides Fail2ban-like protection: tracks abusive behavior
//! (rate limit breaches, brute-force failures) and temporarily bans IPs
//! that exceed configured thresholds within time windows.
//!
//! # Module Integration
//!
//! Other HTTP modules can report abuse events to trigger IP bans via the
//! global abuse recorder (defined in `ferron_http::abuse`). Rate limiting and
//! basic auth modules emit events through the shared trait -- they do not
//! depend on this crate directly.
//!
//! ```ignore
//! use ferron_http::abuse::{AbuseEvent, AbuseEventType, get_global_abuse_recorder};
//!
//! let event = AbuseEvent::new(
//!     AbuseEventType::RateLimitExceeded,
//!     ip_addr,
//!     "Rate limit 100/s exceeded".to_string(),
//!     75,
//! );
//!
//! if let Some(recorder) = get_global_abuse_recorder() {
//!     match recorder.record_event(&event) {
//!         EventResult::BanTriggered => {
//!             // IP is now banned for the configured duration
//!         }
//!         EventResult::Recorded => {
//!             // Event recorded, threshold not yet met
//!         }
//!     }
//! }
//! ```

pub mod config;
pub mod loader;
pub mod registry;
pub mod stage;
pub mod validator;

pub use loader::HttpAbuseProtectionModuleLoader;
pub use registry::AbuseRegistry;
