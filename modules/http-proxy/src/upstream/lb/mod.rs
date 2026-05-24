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

/// Runtime load balancer state.
///
/// Contains the active state for each algorithm (round-robin counter,
/// weighted round-robin state, consistent hash ring, etc.).
#[derive(Clone, Default)]
pub enum LoadBalancerAlgorithmInner {
    Random,
    RoundRobin(WeightedRoundRobinState),
    #[default]
    LeastConnections,
    TwoRandomChoices,
}

impl From<LoadBalancerAlgorithm> for LoadBalancerAlgorithmInner {
    fn from(alg: LoadBalancerAlgorithm) -> Self {
        match alg {
            LoadBalancerAlgorithm::Random => LoadBalancerAlgorithmInner::Random,
            LoadBalancerAlgorithm::RoundRobin => {
                LoadBalancerAlgorithmInner::RoundRobin(WeightedRoundRobinState::new())
            }
            LoadBalancerAlgorithm::LeastConnections => LoadBalancerAlgorithmInner::LeastConnections,
            LoadBalancerAlgorithm::TwoRandomChoices => LoadBalancerAlgorithmInner::TwoRandomChoices,
        }
    }
}
