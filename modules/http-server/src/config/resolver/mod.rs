#![allow(unused_imports)]
#![allow(dead_code)]
#![allow(clippy::module_inception)]
//! Configuration Resolver

mod matcher;
mod resolver;
mod types;

// Re-export public types and the main resolver
pub use matcher::{
    evaluate_matcher_condition, evaluate_matcher_conditions, resolve_matcher_operand,
    CompiledMatcherExpr,
};
pub use resolver::ThreeStageResolver;
pub use types::{ResolutionResult, ResolvedLocationPath};
