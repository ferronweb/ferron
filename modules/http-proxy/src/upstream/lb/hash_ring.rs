//! Ketama-style consistent hash ring for backend selection.

use std::hash::Hasher;

use crate::types::upstream::UpstreamInner;

/// Ketama-style consistent hash ring for backend selection.
#[derive(Clone, Debug)]
pub struct ConsistentHashRing {
    nodes: Vec<(u64, usize)>,
    backend_count: usize,
    weights_hash: u64,
}

impl ConsistentHashRing {
    const VNODES_PER_BACKEND: usize = 160;

    pub fn new(backends: &[UpstreamInner]) -> Self {
        let (nodes, weights_hash) = Self::build_nodes(backends);
        Self {
            nodes,
            backend_count: backends.len(),
            weights_hash,
        }
    }

    fn build_nodes(backends: &[UpstreamInner]) -> (Vec<(u64, usize)>, u64) {
        let total_vnodes: usize = backends
            .iter()
            .map(|b| Self::VNODES_PER_BACKEND * b.weight as usize)
            .sum();
        let mut nodes = Vec::with_capacity(total_vnodes);
        let mut weights_hash = 0u64;

        for (idx, backend) in backends.iter().enumerate() {
            weights_hash = weights_hash
                .wrapping_mul(31)
                .wrapping_add(backend.weight as u64);

            let vnode_count = Self::VNODES_PER_BACKEND * backend.weight as usize;
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

    pub fn get(&self, key: &[u8]) -> Option<usize> {
        if self.nodes.is_empty() {
            return None;
        }

        let mut h = crate::upstream::get_ahasher();
        h.write(key);
        let hash = h.finish();

        match self.nodes.binary_search_by_key(&hash, |(h, _)| *h) {
            Ok(idx) => Some(self.nodes[idx].1),
            Err(idx) => {
                if idx < self.nodes.len() {
                    Some(self.nodes[idx].1)
                } else {
                    Some(self.nodes[0].1)
                }
            }
        }
    }

    pub fn needs_rebuild(&self, backends: &[UpstreamInner]) -> bool {
        if self.backend_count != backends.len() {
            return true;
        }
        let hash = backends.iter().fold(0u64, |h, b| {
            h.wrapping_mul(31).wrapping_add(b.weight as u64)
        });
        self.weights_hash != hash
    }

    pub fn rebuild(&mut self, backends: &[UpstreamInner]) {
        let (nodes, weights_hash) = Self::build_nodes(backends);
        self.nodes = nodes;
        self.backend_count = backends.len();
        self.weights_hash = weights_hash;
    }
}
