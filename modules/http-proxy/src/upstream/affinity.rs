//! Session affinity (sticky session) implementation.

use std::sync::Arc;

use parking_lot::RwLock;

use crate::{types::upstream::UpstreamInner, upstream::lb::ConsistentHashRing};

/// Resolve an affinity key to a backend index.
///
/// For cookie and header affinity, the key is a backend identifier
/// (hash of the upstream URL). For IP and hash affinity, the key
/// is used directly with the consistent hash ring.
#[inline]
pub fn resolve_affinity_index(
    affinity_type: &crate::types::affinity::AffinityType,
    affinity_key: &[u8],
    backends: &[Arc<UpstreamInner>],
    ring: &RwLock<ConsistentHashRing>,
) -> Option<usize> {
    if backends.is_empty() {
        return None;
    }

    match affinity_type {
        crate::types::affinity::AffinityType::Cookie(_)
        | crate::types::affinity::AffinityType::Header(_)
        | crate::types::affinity::AffinityType::Ip
        | crate::types::affinity::AffinityType::Hash { .. } => {
            let mut guard = ring.upgradable_read();
            if guard.needs_rebuild(backends) {
                guard.with_upgraded(|g| {
                    if !g.needs_rebuild(backends) {
                        // The ring is already up-to-date, no need to rebuild.
                        return;
                    }
                    g.rebuild(backends);
                })
            }
            guard.get(affinity_key)
        }
    }
}
