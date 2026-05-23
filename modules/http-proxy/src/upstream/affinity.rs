//! Session affinity (sticky session) implementation.

use std::hash::Hasher;

use crate::types::upstream::UpstreamInner;

/// Resolve an affinity key to a backend index.
///
/// For cookie and header affinity, the key is a backend identifier
/// (hash of the upstream URL). For IP and hash affinity, the key
/// is used directly with the consistent hash ring.
pub fn resolve_affinity_index(
    affinity_type: &crate::types::affinity::AffinityType,
    affinity_key: &[u8],
    backends: &[UpstreamInner],
    algorithm: &super::lb::LoadBalancerAlgorithmInner,
) -> Option<usize> {
    if backends.is_empty() {
        return None;
    }

    match affinity_type {
        crate::types::affinity::AffinityType::Cookie(_)
        | crate::types::affinity::AffinityType::Header(_) => {
            // For cookie/header affinity, the key is a backend identifier.
            // We try to match it against each backend's identifier.
            let key_str = std::str::from_utf8(affinity_key).ok()?;
            backends
                .iter()
                .position(|b| super::backend_affinity_id(b) == key_str)
        }
        crate::types::affinity::AffinityType::Ip
        | crate::types::affinity::AffinityType::Hash { .. } => {
            // For IP and hash affinity, use consistent hashing.
            let ring = match algorithm {
                super::lb::LoadBalancerAlgorithmInner::ConsistentHash(ring) => ring,
                _ => {
                    // Fall back to simple modulus hashing
                    let mut h = super::get_ahasher();
                    h.write(affinity_key);
                    let hash = h.finish();
                    return Some((hash as usize) % backends.len());
                }
            };
            let guard = ring.read();
            guard.get(affinity_key)
        }
    }
}

/// Generate a short affinity identifier for a backend.
///
/// Uses the first 8 hex characters of the upstream URL's ahash.
pub fn backend_affinity_id(backend: &UpstreamInner) -> String {
    let mut h = super::get_ahasher();
    h.write(backend.proxy_to.as_bytes());
    let hash = h.finish();
    format!("{hash:016x}")
}
