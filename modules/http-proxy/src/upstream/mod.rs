//! Upstream resolution and load balancing logic.
//!
//! This module re-exports types from `crate::types` for backward compatibility,
//! and provides upstream-specific functions (affinity, circuit breaker, resolution).

pub mod affinity;
pub mod circuit;
pub mod failure_cache;
pub mod lb;
pub mod resolution;

#[cfg(test)]
pub mod tests;

use std::hash::BuildHasher;

// Re-export upstream-specific functions
pub use circuit::{
    is_circuit_breaker_available, record_backend_response, record_backend_transport_failure,
};
pub use failure_cache::ConcurrentTtlCache;
pub(crate) use failure_cache::FailureCache;
pub use resolution::{determine_proxy_to, resolve_upstreams};

/// Returns an [`ahash::AHasher`] with a consistent seed.
///
/// This is used for deterministic hashing of affinity keys,
/// so that the same key always maps to the same backend.
#[inline]
pub fn get_ahasher() -> ahash::AHasher {
    // Hard-coded seed values to ensure consistent hashing across deployments.
    ahash::RandomState::with_seeds(
        0x0f1fdc6efcc97fd9,
        0x942bd4a9d2ec6246,
        0xcf8d27c1af157eb4,
        0xda2d3937288cc846,
    )
    .build_hasher()
}
