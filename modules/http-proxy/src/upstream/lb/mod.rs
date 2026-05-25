//! Load balancer runtime state.
//!
//! The configuration types (`LoadBalancerAlgorithm`, `SelectedBackend`)
//! are defined in `crate::types` to avoid circular dependencies.

pub mod hash_ring;
pub mod p2c_ewma;
pub mod round_robin;
pub mod selector;

pub use crate::types::lb::LoadBalancerAlgorithm;
pub use hash_ring::ConsistentHashRing;
pub use p2c_ewma::EwmaStateMap;
pub use round_robin::WeightedRoundRobinState;

/// Runtime load balancer state.
///
/// Contains the active state for each algorithm (round-robin counter,
/// weighted round-robin state, consistent hash ring, etc.).
///
/// The P2C+EWMA algorithm's per-backend latency state lives separately
/// in [`EwmaStateMap`], shared from `ProxyState`.
#[derive(Clone, Default)]
pub enum LoadBalancerAlgorithmInner {
    Random,
    RoundRobin(WeightedRoundRobinState),
    #[default]
    LeastConnections,
    TwoRandomChoices,
    P2cEwma,
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
            LoadBalancerAlgorithm::P2cEwma => LoadBalancerAlgorithmInner::P2cEwma,
        }
    }
}
