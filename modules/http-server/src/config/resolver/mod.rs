#![allow(unused_imports)]
#![allow(dead_code)]
//! 3-Stage Configuration Resolver
//!
//! This module provides a modular configuration resolution system with three independent stages:
//!
//! 1. **Stage 1** - IP address-based resolution (BTreeMap)
//! 2. **Stage 2** - Main resolution using radix tree (hostname segments, wildcards, path segments, conditionals)
//! 3. **Stage 3** - Error configuration resolution (HashMap)
//!
//! Each stage can be used independently or composed together via the main resolver.

mod matcher;
mod resolver;
mod stage1;
mod stage2;
mod stage3;
mod types;

// Re-export public types and the main resolver
pub use matcher::{
    evaluate_matcher_condition, evaluate_matcher_conditions, resolve_matcher_operand,
    CompiledMatcherExpr,
};
pub use resolver::ThreeStageResolver;
pub use stage1::Stage1IpResolver;
pub use stage2::{RadixKey, RadixNodeData, Stage2RadixResolver};
pub use stage3::{ErrorConfigScope, ErrorConfigScopeKey, Stage3ErrorResolver};
pub use types::{ResolutionResult, ResolvedLocationPath, ResolverVariables};
