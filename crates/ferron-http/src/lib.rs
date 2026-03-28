//! HTTP types and server implementation for Ferron
//!
//! This crate provides HTTP-specific types including HttpContext and
//! ready-to-use HTTP module implementations.

mod context;
mod loader;
mod server;
mod stages;

pub use context::HttpContext;
pub use loader::BasicHttpModuleLoader;
pub use server::BasicHttpModule;
pub use stages::{HelloStage, LoggingStage, NotFoundStage};
