//! Shared types for the http-proxy module.
//!
//! This module contains all the core types used by both `proxy/` and `upstream/`.
//! By keeping these types at the crate root level, we avoid circular dependencies
//! between the proxy logic and upstream resolution logic.

pub mod affinity;
pub mod circuit;
pub mod error;
pub mod health;
pub mod lb;
pub mod srv;
#[cfg(feature = "srv-lookup")]
pub mod strict_dns;
pub mod upstream;

/// Shared connection tracking state for least-conn and two-random algorithms.
///
/// Maps upstream keys to `Arc<()>` trackers that are cloned to count
/// active connections per backend.
pub type ConnectionsTrackState = std::sync::Arc<
    dashmap::DashMap<
        std::sync::Arc<self::upstream::UpstreamInner>,
        std::sync::Arc<()>,
        rustc_hash::FxBuildHasher,
    >,
>;
