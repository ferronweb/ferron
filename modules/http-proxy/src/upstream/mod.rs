//! Upstream resolution and load balancing logic.

pub mod affinity;
pub mod circuit;
pub mod lb;
pub mod resolution;
pub mod types;

#[cfg(test)]
pub mod tests;

use std::hash::BuildHasher;

// Re-export key types for convenience
pub use types::affinity::{AffinityConfig, AffinityType, HashMethod};
pub use types::circuit::CircuitBreakerStateMap;
pub use types::health::{ExpectedStatusCodes, HealthCheckMethod, UpstreamHealthCheckConfig};
pub use types::lb::LoadBalancerAlgorithm;
pub use types::upstream::ProxyHeader;
pub use types::*;

// Re-export functions
pub use affinity::backend_affinity_id;
pub use circuit::{
    is_circuit_breaker_available, record_backend_response, record_backend_transport_failure,
};
pub use resolution::determine_proxy_to;
pub use resolution::resolve_upstreams;
pub use types::upstream::SrvUpstreamData;

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
