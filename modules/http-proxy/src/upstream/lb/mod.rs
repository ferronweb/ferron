//! Load balancer runtime state.
//!
//! The configuration types (`LoadBalancerAlgorithm`, `SelectedBackend`)
//! are defined in `crate::types` to avoid circular dependencies.

pub mod hash_ring;
pub mod round_robin;
pub mod selector;

pub use crate::types::lb::LoadBalancerAlgorithm;
pub use hash_ring::ConsistentHashRing;
pub use round_robin::WeightedRoundRobinState;

use parking_lot::{Mutex, RwLock};
use std::sync::Arc;

/// Runtime load balancer state.
///
/// Contains the active state for each algorithm (round-robin counter,
/// weighted round-robin state, consistent hash ring, etc.).
#[derive(Clone, Default)]
pub enum LoadBalancerAlgorithmInner {
    Random,
    RoundRobin(Arc<std::sync::atomic::AtomicUsize>),
    #[default]
    LeastConnections,
    TwoRandomChoices,
    WeightedRoundRobin(Arc<Mutex<WeightedRoundRobinState>>),
    ConsistentHash(Arc<RwLock<ConsistentHashRing>>),
}

impl From<LoadBalancerAlgorithm> for LoadBalancerAlgorithmInner {
    fn from(alg: LoadBalancerAlgorithm) -> Self {
        match alg {
            LoadBalancerAlgorithm::Random => LoadBalancerAlgorithmInner::Random,
            LoadBalancerAlgorithm::RoundRobin => LoadBalancerAlgorithmInner::RoundRobin(Arc::new(
                std::sync::atomic::AtomicUsize::new(0),
            )),
            LoadBalancerAlgorithm::LeastConnections => LoadBalancerAlgorithmInner::LeastConnections,
            LoadBalancerAlgorithm::TwoRandomChoices => LoadBalancerAlgorithmInner::TwoRandomChoices,
            LoadBalancerAlgorithm::WeightedRoundRobin => {
                LoadBalancerAlgorithmInner::WeightedRoundRobin(Arc::new(Mutex::new(
                    WeightedRoundRobinState::new(),
                )))
            }
            LoadBalancerAlgorithm::ConsistentHash => LoadBalancerAlgorithmInner::ConsistentHash(
                Arc::new(RwLock::new(ConsistentHashRing::new(&[]))),
            ),
        }
    }
}
