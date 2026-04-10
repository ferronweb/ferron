//! HTTP Basic Authentication module for Ferron.
//!
//! Provides the `basic_auth` directive for request-level authentication using
//! hashed passwords (Argon2, PBKDF2, or scrypt). Includes built-in brute-force
//! protection with per-username attempt tracking and automatic lockout.

mod brute_force;
mod config;
mod loader;
mod stage;
mod validator;

pub use loader::HttpBasicAuthModuleLoader;
