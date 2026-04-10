//! Admin API module for Ferron.
//!
//! Provides a separate HTTP server on a configurable port
//! for administrative endpoints (/health, /status, /config, /reload).

mod config;
mod handlers;
mod loader;
mod server;
mod validator;

pub use config::AdminConfig;
pub use loader::AdminApiModuleLoader;
pub use server::AdminApiModule;
