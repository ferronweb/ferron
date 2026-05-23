//! Ketama-style consistent hash ring for backend selection.

use std::hash::Hasher;

use crate::types::upstream::UpstreamInner;

/// Ketama-style consistent hash ring for backend selection.
#[derive(Clone, Debug)]
pub struct ConsistentHashRing {
    nodes: Vec<(u64, usize)>,
    backend_count: usize,
}

impl ConsistentHashRing {
    const VNODES_PER_BACKEND: usize = 160;

    pub fn new(backends: &[UpstreamInner]) -> Self {
        let nodes = Self::build_nodes(backends);
        Self {
            nodes,
            backend_count: backends.len(),
        }
    }

    fn build_nodes(backends: &[UpstreamInner]) -> Vec<(u64, usize)> {
        let mut nodes = Vec::with_capacity(backends.len() * Self::VNODES_PER_BACKEND);

        for (idx, backend) in backends.iter().enumerate() {
            for vnode in 0..Self::VNODES_PER_BACKEND {
                let key = format!("{}#{}", backend.proxy_to, vnode);
                let mut h = crate::upstream::get_ahasher();
                h.write(key.as_bytes());
                let hash = h.finish();
                nodes.push((hash, idx));
            }
        }

        nodes.sort_by_key(|&(hash, _)| hash);
        nodes
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

    pub fn needs_rebuild(&self, backend_count: usize) -> bool {
        self.backend_count != backend_count
    }

    pub fn rebuild(&mut self, backends: &[UpstreamInner]) {
        self.nodes = Self::build_nodes(backends);
        self.backend_count = backends.len();
    }
}
