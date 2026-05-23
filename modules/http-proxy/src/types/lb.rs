//! Load balancer configuration types.
//!
//! These are the configuration-time algorithm choices and related types.

use std::sync::Arc;

/// Load balancing algorithm configuration.
///
/// This enum represents the algorithm choice made at configuration time.
/// The runtime state is managed in [`crate::upstream::lb::LoadBalancerAlgorithmInner`].
#[derive(Clone, Copy, Debug, Default)]
pub enum LoadBalancerAlgorithm {
    /// Random selection.
    Random,
    /// Round-robin cycling.
    RoundRobin,
    /// Least active connections.
    LeastConnections,
    /// Pick two random, select less loaded.
    #[default]
    TwoRandomChoices,
    /// Smooth weighted round-robin load balancing.
    WeightedRoundRobin,
    /// Consistent hashing based on request key.
    ConsistentHash,
}

/// Result of backend selection: the upstream and its connection tracker.
///
/// The `tracker` field is used by load balancing algorithms that need to
/// track active connections per backend (e.g., LeastConnections, TwoRandomChoices).
/// For algorithms like Random and RoundRobin, this is `None`.
pub struct SelectedBackend {
    /// The selected upstream.
    pub upstream: super::upstream::UpstreamInner,
    /// Connection tracker for LeastConnections/TwoRandomChoices.
    /// `None` for Random/RoundRobin algorithms.
    pub tracker: Option<Arc<()>>,
}
