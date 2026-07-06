//! Ketama-style consistent hash ring for backend selection.

use std::hash::Hasher;
use std::sync::Arc;

use rustc_hash::FxHashSet;

use crate::types::upstream::UpstreamInner;

/// Ketama-style consistent hash ring for backend selection.
#[derive(Clone, Debug)]
pub struct ConsistentHashRing {
    nodes: Vec<(u64, usize)>,
    backend_count: usize,
    weights_hash: u64,
}

impl ConsistentHashRing {
    /// Maximum effective weight per backend to prevent memory exhaustion
    /// from `weight * VNODES_PER_BACKEND` allocations.
    /// 100 × 160 = 16,000 vnodes per backend (~256 KiB) is more than
    /// sufficient for good distribution.
    const MAX_EFFECTIVE_WEIGHT: u32 = 100;

    const VNODES_PER_BACKEND: usize = 160;

    #[inline]
    pub fn new(backends: &[Arc<UpstreamInner>]) -> Self {
        let (nodes, weights_hash) = Self::build_nodes(backends);
        Self {
            nodes,
            backend_count: backends.len(),
            weights_hash,
        }
    }

    #[inline]
    fn effective_weight(weight: u32) -> usize {
        (weight.min(Self::MAX_EFFECTIVE_WEIGHT) as usize).saturating_mul(Self::VNODES_PER_BACKEND)
    }

    fn build_nodes(backends: &[Arc<UpstreamInner>]) -> (Vec<(u64, usize)>, u64) {
        let total_vnodes: usize = backends
            .iter()
            .map(|b| Self::effective_weight(b.weight))
            .sum();
        let mut nodes = Vec::with_capacity(total_vnodes);
        let mut weights_hash = 0u64;

        for (idx, backend) in backends.iter().enumerate() {
            weights_hash = weights_hash
                .wrapping_mul(31)
                .wrapping_add(backend.weight as u64);

            let vnode_count = Self::effective_weight(backend.weight);
            for vnode in 0..vnode_count {
                let key = format!("{}#{}", backend.proxy_to, vnode);
                let mut h = crate::upstream::get_ahasher();
                h.write(key.as_bytes());
                let hash = h.finish();
                nodes.push((hash, idx));
            }
        }

        nodes.sort_by_key(|&(hash, _)| hash);
        (nodes, weights_hash)
    }

    #[inline]
    pub fn get(&self, key: &[u8], exclude_idx: &FxHashSet<usize>) -> Option<usize> {
        if self.nodes.is_empty() {
            return None;
        }

        let mut h = crate::upstream::get_ahasher();
        h.write(key);
        let hash = h.finish();

        // 1. O(log N) jump to the first element >= target
        let start_idx = self.nodes.partition_point(|(h, _)| *h < hash);

        // 2. Linear probe forward, skipping any ID found in the exclusion set.
        // This compiles down to a highly optimized loop with excellent cache locality.
        let start_idx_refined = self.nodes[start_idx..]
            .iter()
            .find(|&&(_, id)| !exclude_idx.contains(&id))
            .or_else(|| {
                self.nodes[..start_idx]
                    .iter()
                    .find(|&&(_, id)| !exclude_idx.contains(&id))
            });

        // 3. Return the refined index if found; otherwise return None.
        start_idx_refined.map(|&(_, id)| id)
    }

    #[inline]
    pub fn needs_rebuild(&self, backends: &[Arc<UpstreamInner>]) -> bool {
        if self.backend_count != backends.len() {
            return true;
        }
        let hash = backends.iter().fold(0u64, |h, b| {
            h.wrapping_mul(31).wrapping_add(b.weight as u64)
        });
        self.weights_hash != hash
    }

    #[inline]
    pub fn rebuild(&mut self, backends: &[Arc<UpstreamInner>]) {
        let (nodes, weights_hash) = Self::build_nodes(backends);
        self.nodes = nodes;
        self.backend_count = backends.len();
        self.weights_hash = weights_hash;
    }

    #[cfg(feature = "fuzz")]
    #[inline]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn make_upstream(url: &str) -> Arc<UpstreamInner> {
        Arc::new(UpstreamInner {
            proxy_to: url.to_string(),
            connect_to: None,
            proxy_unix: None,
            weight: 1,
            mtls: None,
            priority: 0,
            connection_timeout: None,
            idle_timeout: std::time::Duration::from_secs(60),
        })
    }

    fn make_upstream_with_weight(url: &str, weight: u32) -> Arc<UpstreamInner> {
        Arc::new(UpstreamInner {
            proxy_to: url.to_string(),
            connect_to: None,
            proxy_unix: None,
            weight,
            mtls: None,
            priority: 0,
            connection_timeout: None,
            idle_timeout: std::time::Duration::from_secs(60),
        })
    }

    #[test]
    fn test_consistent_hash_ring_basic() {
        let backends = vec![
            make_upstream("http://backend1"),
            make_upstream("http://backend2"),
            make_upstream("http://backend3"),
        ];
        let ring = ConsistentHashRing::new(&backends);

        // Same key should always map to the same backend
        let key1 = b"test-key-1";
        let idx1 = ring.get(key1, &FxHashSet::default()).unwrap();
        let idx2 = ring.get(key1, &FxHashSet::default()).unwrap();
        assert_eq!(idx1, idx2);

        // Excluded backend should be redirected to another backend
        let idx3 = ring.get(key1, &FxHashSet::from_iter([idx1])).unwrap();
        assert_ne!(idx1, idx3);

        // Different keys may map to different backends
        let key2 = b"test-key-2";
        let idx3 = ring.get(key2, &FxHashSet::default()).unwrap();
        assert!(idx3 < backends.len());
    }

    #[test]
    fn test_consistent_hash_ring_all_backends_reachable() {
        let backends = vec![
            make_upstream("http://backend1"),
            make_upstream("http://backend2"),
            make_upstream("http://backend3"),
        ];
        let ring = ConsistentHashRing::new(&backends);

        // Try many keys to ensure all backends are reachable
        let mut seen = [false; 3];
        for i in 0..1000 {
            let key = format!("key-{i}");
            if let Some(idx) = ring.get(key.as_bytes(), &FxHashSet::default()) {
                seen[idx] = true;
            }
        }

        // All backends should be reachable with enough keys
        assert!(seen.iter().all(|&s| s), "Not all backends were reachable");
    }

    #[test]
    fn test_consistent_hash_ring_empty() {
        let backends: Vec<Arc<UpstreamInner>> = vec![];
        let ring = ConsistentHashRing::new(&backends);
        assert!(ring.get(b"test", &FxHashSet::default()).is_none());
    }

    #[test]
    fn test_consistent_hash_ring_rebuild() {
        let backends = vec![
            make_upstream("http://backend1"),
            make_upstream("http://backend2"),
        ];
        let mut ring = ConsistentHashRing::new(&backends);

        assert!(!ring.needs_rebuild(&backends));

        let three_backends = vec![
            make_upstream("http://backend1"),
            make_upstream("http://backend2"),
            make_upstream("http://backend3"),
        ];
        assert!(ring.needs_rebuild(&three_backends));

        ring.rebuild(&three_backends);
        assert!(!ring.needs_rebuild(&three_backends));
    }

    #[test]
    fn test_consistent_hash_ring_same_no_rebuild() {
        let three_backends = vec![
            make_upstream("http://backend1"),
            make_upstream("http://backend2"),
            make_upstream("http://backend3"),
        ];
        let ring = ConsistentHashRing::new(&three_backends);

        assert!(!ring.needs_rebuild(&three_backends));

        let three_backends = vec![
            make_upstream("http://backend1"),
            make_upstream("http://backend2"),
            make_upstream("http://backend3"),
        ];
        assert!(!ring.needs_rebuild(&three_backends));
    }

    #[test]
    fn test_consistent_hash_ring_weighted_distribution() {
        let backends = vec![
            make_upstream_with_weight("http://heavy", 3),
            make_upstream_with_weight("http://light", 1),
        ];
        let ring = ConsistentHashRing::new(&backends);

        // With weights 3:1, the heavy backend should get ~75% of keys
        let total = 10_000;
        let mut heavy_count = 0;
        for i in 0..total {
            let key = format!("key-{i}");
            if let Some(idx) = ring.get(key.as_bytes(), &FxHashSet::default()) {
                if idx == 0 {
                    heavy_count += 1;
                }
            }
        }

        let ratio = heavy_count as f64 / total as f64;
        assert!(
            (0.70..0.80).contains(&ratio),
            "Expected ~75% for weight-3 backend, got {:.2}%",
            ratio * 100.0
        );
    }
}
