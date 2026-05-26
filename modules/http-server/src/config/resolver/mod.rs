//! Configuration Resolver

mod matcher;
#[allow(clippy::module_inception)]
mod resolver;
#[cfg(test)]
mod tests;
mod tree;
mod types;

// Re-export public types and the main resolver
pub use resolver::ThreeStageResolver;
#[allow(unused_imports)]
pub use types::{ResolutionResult, ResolvedLocationPath};
