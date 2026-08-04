//! Load balancer configuration types.
//!
//! These are the configuration-time algorithm choices and related types.

/// Load balancing algorithm configuration.
///
/// This enum represents the algorithm choice made at configuration time.
/// The runtime state is managed in [`crate::upstream::lb::LoadBalancerAlgorithmInner`].
#[derive(Clone, Copy, Debug, Default)]
pub enum LoadBalancerAlgorithm {
    /// Random selection.
    Random,
    /// Smooth weighted round-robin load balancing.
    RoundRobin,
    /// Least active connections.
    LeastConnections,
    /// Pick two random, select less loaded.
    #[default]
    TwoRandomChoices,
    /// Power of Two Choices with EWMA latency scoring.
    /// Picks two random backends and selects the one with the lower combined
    /// score of EWMA response latency + active connection penalty.
    P2cEwma,
}
