//! Ketama-style consistent hash ring for canary variant selection.

use std::hash::{BuildHasher, Hasher};

use crate::config::CanaryVariant;

/// Maximum effective weight per variant to prevent memory exhaustion
/// from `weight * VNODES_PER_VARIANT` allocations.
/// 100 × 160 = 16,000 vnodes per variant (~256 KiB) is more than
/// sufficient for good distribution.
const MAX_EFFECTIVE_WEIGHT: u32 = 100;

const VNODES_PER_VARIANT: usize = 160;

/// Returns an `AHasher` with a consistent seed.
///
/// This produces deterministic hashing of affinity keys, so that the same
/// key always maps to the same variant across processes and reloads.
#[inline]
pub fn get_ahasher() -> ahash::AHasher {
    // Hard-coded seed values, matching the reverse proxy consistent hash ring.
    ahash::RandomState::with_seeds(
        0x0f1fdc6efcc97fd9,
        0x942bd4a9d2ec6246,
        0xcf8d27c1af157eb4,
        0xda2d3937288cc846,
    )
    .build_hasher()
}

/// Ketama-style consistent hash ring over named variants.
///
/// Each variant contributes a number of virtual nodes proportional to its
/// weight. Keys are hashed onto the ring and served by the variant whose
/// virtual node follows their hash.
///
/// Consistent hashing keeps existing assignments stable when the
/// configuration changes: adjusting a variant's weight only moves the keys
/// whose nearest virtual node was added or removed, so preferences survive
/// weight changes and configuration reloads.
#[derive(Clone, Debug)]
pub struct VariantRing {
    nodes: Vec<(u64, usize)>,
}

impl VariantRing {
    #[inline]
    fn effective_vnodes(weight: u32) -> usize {
        (weight.min(MAX_EFFECTIVE_WEIGHT) as usize).saturating_mul(VNODES_PER_VARIANT)
    }

    /// Build a ring from the given variants. Heavier variants receive
    /// proportionally more virtual nodes.
    pub fn new(variants: &[CanaryVariant]) -> Self {
        let total_vnodes: usize = variants
            .iter()
            .map(|v| Self::effective_vnodes(v.weight))
            .sum();
        let mut nodes = Vec::with_capacity(total_vnodes);

        for (idx, variant) in variants.iter().enumerate() {
            let vnode_count = Self::effective_vnodes(variant.weight);
            for vnode in 0..vnode_count {
                let key = format!("{}#{}", variant.name, vnode);
                let mut h = get_ahasher();
                h.write(key.as_bytes());
                nodes.push((h.finish(), idx));
            }
        }

        nodes.sort_by_key(|&(hash, _)| hash);

        Self { nodes }
    }

    /// Map a key to a variant index.
    ///
    /// Returns `None` when the ring has no virtual nodes.
    #[inline]
    pub fn get(&self, key: &[u8]) -> Option<usize> {
        if self.nodes.is_empty() {
            return None;
        }

        let mut h = get_ahasher();
        h.write(key);
        let hash = h.finish();

        // O(log N) jump to the first node >= target, wrapping around.
        let start_idx = self.nodes.partition_point(|(h, _)| *h < hash);
        let idx = if start_idx < self.nodes.len() {
            start_idx
        } else {
            0
        };

        Some(self.nodes[idx].1)
    }

    #[cfg(test)]
    #[inline]
    fn len(&self) -> usize {
        self.nodes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[inline]
    fn make_variant(name: &str, weight: u32) -> CanaryVariant {
        CanaryVariant {
            name: name.to_string(),
            weight,
        }
    }

    #[test]
    fn test_ring_basic() {
        let ring = VariantRing::new(&[
            make_variant("stable", 1),
            make_variant("new", 1),
            make_variant("experimental", 1),
        ]);

        // Same key always maps to the same variant
        let key1 = b"test-key-1";
        let idx1 = ring.get(key1).unwrap();
        let idx2 = ring.get(key1).unwrap();
        assert_eq!(idx1, idx2);

        // All variants are reachable with enough keys
        let mut seen = [false; 3];
        for i in 0..1000 {
            let key = format!("key-{i}");
            seen[ring.get(key.as_bytes()).unwrap()] = true;
        }
        assert!(seen.iter().all(|&s| s), "Not all variants were reachable");
    }

    #[test]
    fn test_ring_empty() {
        let ring = VariantRing::new(&[]);
        assert!(ring.get(b"test").is_none());
        assert_eq!(ring.len(), 0);
    }

    #[test]
    fn test_ring_vnode_count_capped() {
        let ring = VariantRing::new(&[make_variant("huge", 500)]);
        assert_eq!(
            ring.len(),
            MAX_EFFECTIVE_WEIGHT as usize * VNODES_PER_VARIANT
        );
    }

    #[test]
    fn test_ring_weighted_distribution() {
        let ring = VariantRing::new(&[make_variant("heavy", 3), make_variant("light", 1)]);

        // With weights 3:1, the heavy variant should get ~75% of keys
        let total = 10_000;
        let mut heavy_count = 0;
        for i in 0..total {
            let key = format!("key-{i}");
            if ring.get(key.as_bytes()).unwrap() == 0 {
                heavy_count += 1;
            }
        }

        let ratio = heavy_count as f64 / total as f64;
        assert!(
            (0.70..0.80).contains(&ratio),
            "Expected ~75% for weight-3 variant, got {:.2}%",
            ratio * 100.0
        );
    }

    #[test]
    fn test_ring_rebuilt_identical_for_same_config() {
        let variants = vec![make_variant("stable", 90), make_variant("new", 10)];
        let ring_a = VariantRing::new(&variants);
        let ring_b = VariantRing::new(&variants);

        for i in 0..10_000 {
            let key = format!("key-{i}");
            assert_eq!(
                ring_a.get(key.as_bytes()),
                ring_b.get(key.as_bytes()),
                "ring mapping changed for the same configuration"
            );
        }
    }

    #[test]
    fn test_ring_weight_change_moves_only_boundary_keys() {
        let ring_original =
            VariantRing::new(&[make_variant("stable", 90), make_variant("new", 10)]);
        let ring_modified =
            VariantRing::new(&[make_variant("stable", 85), make_variant("new", 15)]);

        let total = 20_000;
        let mut changed = 0usize;
        for i in 0..total {
            let key = format!("key-{i}");
            if ring_original.get(key.as_bytes()) != ring_modified.get(key.as_bytes()) {
                changed += 1;
            }
        }

        // Only the clients whose nearest virtual node was removed flip; the
        // rest keep their variant. The expected flip rate tracks the weight
        // delta (~5%). Assert far away from both 0% and 100%.
        let ratio = changed as f64 / total as f64;
        assert!(
            (0.01..0.15).contains(&ratio),
            "Expected ~5% of keys to move on a 5-point weight change, got {:.2}%",
            ratio * 100.0
        );
    }

    #[test]
    fn test_ring_large_weight_change_remaps_proportional_share() {
        let ring_original =
            VariantRing::new(&[make_variant("stable", 50), make_variant("new", 50)]);
        let ring_modified =
            VariantRing::new(&[make_variant("stable", 90), make_variant("new", 10)]);

        let total = 20_000;
        let mut changed = 0usize;
        for i in 0..total {
            let key = format!("key-{i}");
            if ring_original.get(key.as_bytes()) != ring_modified.get(key.as_bytes()) {
                changed += 1;
            }
        }

        // Moving from 50/50 to 90/10 remaps roughly half the clients (the
        // share proportional to the weight delta of the changed variant).
        let ratio = changed as f64 / total as f64;
        assert!(
            (0.25..0.70).contains(&ratio),
            "Expected ~40% of keys to move on a 40-point weight change, got {:.2}%",
            ratio * 100.0
        );
    }
}
