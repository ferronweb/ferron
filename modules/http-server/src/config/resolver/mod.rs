#![allow(dead_code)]
#![allow(clippy::module_inception)]
//! Configuration Resolver

mod matcher;
mod resolver;
#[cfg(test)]
mod tests;
mod tree;
mod types;

// Re-export public types and the main resolver
pub use resolver::ThreeStageResolver;
#[allow(unused_imports)]
pub use types::{ResolutionResult, ResolvedLocationPath};
