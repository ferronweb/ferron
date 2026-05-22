//! Re-exports for all upstream types.

pub mod affinity;
pub mod circuit;
pub mod health;
pub mod lb;
pub mod upstream;

// Re-export from submodules
pub use health::HealthCheckState;
pub use upstream::{Upstream, UpstreamConfig, UpstreamInner};

/// Shared connection tracking state for least-conn and two-random algorithms.
///
/// Maps upstream keys to `Arc<()>` trackers that are cloned to count
/// active connections per backend.
pub type ConnectionsTrackState =
    std::sync::Arc<dashmap::DashMap<crate::upstream::UpstreamInner, std::sync::Arc<()>>>;

/// Health check state map keyed by upstream URL string.
pub type HealthCheckStateMap =
    std::sync::Arc<dashmap::DashMap<String, crate::upstream::HealthCheckState>>;
