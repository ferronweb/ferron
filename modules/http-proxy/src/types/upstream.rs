//! Core upstream types: connection keys, proxy protocol, and health check configuration.

use std::sync::Arc;
use std::time::Duration;

use crate::types::health::HealthCheckStateMap;

/// Upstream connection key.
///
/// This uniquely identifies a backend server for connection pooling and health tracking.
/// It combines the target URL and optional Unix socket path.
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct UpstreamInner {
    /// Target URL (e.g. `http://localhost:8080/path`).
    pub proxy_to: String,
    /// Optional Unix socket path for local backends.
    pub proxy_unix: Option<String>,
    /// Weight for weighted load balancing algorithms (default 1).
    pub weight: u32,
}

/// Proxy protocol version to send to backends.
#[derive(Clone, Copy, Debug)]
pub enum ProxyHeader {
    /// HAProxy PROXY protocol v1 (text-based).
    V1,
    /// HAProxy PROXY protocol v2 (binary).
    V2,
}

/// Configured upstream backend.
#[derive(Clone, Debug)]
pub struct UpstreamConfig {
    /// Target URL.
    pub url: String,
    /// Optional Unix socket path.
    pub unix_socket: Option<String>,
    /// Per-upstream connection limit.
    pub limit: Option<usize>,
    /// Idle keep-alive timeout. Populated into `ProxyConfig::idle_timeout_map`
    /// during parsing for O(1) lookup at request time.
    #[allow(dead_code)]
    pub idle_timeout: Option<Duration>,
    /// Active health check configuration for this upstream.
    pub health_check_config: crate::types::health::UpstreamHealthCheckConfig,
    /// Weight for weighted load balancing algorithms (default 1).
    pub weight: u32,
}

/// Data for an SRV-based upstream.
///
/// The DNS resolver and runtime handle are obtained lazily at resolution time
/// from the globally-captured secondary runtime handle.
#[cfg(feature = "srv-lookup")]
#[derive(Clone)]
pub struct SrvUpstreamData {
    /// SRV record name (e.g. `_http._tcp.example.com`).
    pub srv_name: String,
    /// Custom DNS servers (empty = use system resolver).
    pub dns_servers: Vec<std::net::IpAddr>,
    /// Per-upstream connection limit.
    pub limit: Option<usize>,
    /// Idle keep-alive timeout.
    #[allow(dead_code)]
    pub idle_timeout: Option<Duration>,
    /// Active health check configuration for this upstream.
    pub health_check_config: crate::types::health::UpstreamHealthCheckConfig,
    /// Weight for weighted load balancing algorithms (default 1).
    pub weight: u32,
}

/// An upstream backend — either a static URL or an SRV record.
#[derive(Clone)]
pub enum Upstream {
    /// Static upstream with a fixed URL and configuration.
    Static(UpstreamConfig),
    /// SRV-based upstream resolved via DNS.
    #[cfg(feature = "srv-lookup")]
    Srv(SrvUpstreamData),
}

impl Upstream {
    /// Resolve this upstream to a list of concrete `UpstreamInner` entries.
    ///
    /// Static upstreams return themselves. SRV upstreams perform a DNS lookup
    /// on the secondary Tokio runtime, filter unhealthy backends, and perform
    /// weighted random selection within the highest-priority group.
    #[inline]
    pub async fn resolve(
        &self,
        _failed_backends: std::sync::Arc<
            parking_lot::RwLock<crate::util::TtlCache<Arc<UpstreamInner>, u64>>,
        >,
        _health_check_max_fails: u64,
        _active_health_check_state: Option<HealthCheckStateMap>,
    ) -> Vec<Arc<UpstreamInner>> {
        match self {
            Upstream::Static(cfg) => vec![Arc::new(UpstreamInner {
                proxy_to: cfg.url.clone(),
                proxy_unix: cfg.unix_socket.clone(),
                weight: cfg.weight,
            })],
            #[cfg(feature = "srv-lookup")]
            Upstream::Srv(srv_data) => {
                super::srv::resolve_srv(
                    srv_data,
                    _failed_backends,
                    _health_check_max_fails,
                    _active_health_check_state,
                )
                .await
            }
        }
    }
}
