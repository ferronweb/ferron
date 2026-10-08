//! Upstream resolution and load balancing logic.
//!
//! This module re-exports types from `crate::types` and provides upstream-specific
//! functions (affinity, circuit breaker, resolution).

pub mod affinity;
pub mod circuit;
pub mod flapping;
pub mod lb;
pub mod resolution;

#[cfg(test)]
pub mod tests;

use ferron_observability::LogAttributeValue;
use std::hash::BuildHasher;

// Re-export upstream-specific functions
pub use circuit::{record_backend_response, record_backend_transport_failure};
pub use resolution::{resolve_upstreams, BackendSet};

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

/// Obtains log attributes and upstream log ID for health checking
#[inline]
pub fn health_upstream_attrs(
    upstream: &std::sync::Arc<crate::types::upstream::ResolvedUpstream>,
) -> (Vec<(&'static str, LogAttributeValue)>, String) {
    let mut health_attrs = Vec::with_capacity(8);
    let upstream_url = upstream.proxy_to.clone();
    health_attrs.push((
        "upstream.address",
        LogAttributeValue::String(upstream_url.clone()),
    ));
    health_attrs.push((
        "ferron.proxy.backend_url",
        LogAttributeValue::String(upstream_url.clone()),
    ));
    if let Some(ref unix_path) = upstream.proxy_unix {
        health_attrs.push((
            "ferron.proxy.backend_unix_path",
            LogAttributeValue::String(unix_path.clone()),
        ));
    }
    if let Some(ref connect_to) = upstream.connect_to {
        health_attrs.push((
            "ferron.proxy.backend_resolved_ip",
            LogAttributeValue::String(connect_to.to_string()),
        ));
    }
    let upstream_log_id = if let Some(ref connect_to) = upstream.connect_to {
        format!("{upstream_url} (at {connect_to})")
    } else {
        upstream_url.to_owned()
    };

    (health_attrs, upstream_log_id)
}
