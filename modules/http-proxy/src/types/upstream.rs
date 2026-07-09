//! Core upstream types: connection keys, proxy protocol, and health check configuration.

use std::{net::SocketAddr, sync::Arc};

use crate::types::health::HealthCheckStateMap;

/// DNS resolution status for an upstream backend.
///
/// Tracks whether DNS resolution was applicable and its outcome,
/// used for metric labeling when `metrics_resolved_ip` is enabled.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum DnsResolutionStatus {
    /// DNS resolution not applicable (static URL, IP literal, Unix socket).
    #[default]
    NotApplicable,
    /// DNS resolution succeeded (`connect_to` contains the resolved IP).
    Resolved,
    /// DNS lookup failed — domain not found (NXDOMAIN).
    Nxdomain,
    /// DNS lookup failed — other error (SERVFAIL, timeout, etc.).
    DnsError,
    /// Logical DNS mode — resolution deferred to TCP connect time.
    LogicalDns,
}

impl DnsResolutionStatus {
    /// Returns the metric label value for this status.
    pub fn as_label(&self) -> &'static str {
        match self {
            Self::NotApplicable => "static",
            Self::Resolved => "resolved",
            Self::Nxdomain => "nxdomain",
            Self::DnsError => "dns_error",
            Self::LogicalDns => "logical_dns",
        }
    }
}

/// Upstream connection key.
///
/// This uniquely identifies a backend server for connection pooling and health tracking.
/// It combines the target URL and optional Unix socket path.
#[derive(Clone, Eq, PartialEq, Debug)]
pub struct UpstreamInner {
    /// Target URL (e.g. `http://localhost:8080/path`).
    pub proxy_to: String,
    /// Pre-resolved IP address for TCP connection (e.g. `1.2.3.4:8080`).
    ///
    /// When set, the connection layer uses this address for the TCP connect
    /// instead of resolving the hostname from `proxy_to`. The original hostname
    /// in `proxy_to` is still used for TLS SNI and the HTTP request URI.
    ///
    /// Set by strict DNS resolution (A/AAAA records). `None` for static URLs,
    /// IP literals, Unix sockets, and logical DNS mode.
    pub connect_to: Option<SocketAddr>,
    /// Optional Unix socket path for local backends.
    pub proxy_unix: Option<String>,
    /// Weight for weighted load balancing algorithms (default 1).
    pub weight: u32,
    /// mTLS credentials for an upstream
    pub mtls: Option<Arc<MtlsCredentials>>,
    /// Priority for tiered failover. Lower values = higher priority.
    /// Backends at the highest-priority tier are tried first; lower-priority
    /// tiers are used as fallbacks when all higher-priority backends are
    /// unavailable. Default: 0 (highest priority).
    pub priority: u16,
    /// Connection timeout for this upstream
    pub connection_timeout: Option<std::time::Duration>,
    /// Idle timeout for keepalive connections to this upstream.
    pub idle_timeout: std::time::Duration,
    /// DNS resolution status, used for metric labeling.
    pub dns_status: DnsResolutionStatus,
}

impl std::hash::Hash for UpstreamInner {
    #[inline]
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (
            &self.proxy_to,
            &self.connect_to,
            &self.proxy_unix,
            &self.mtls,
        )
            .hash(state);
    }
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
    /// Active health check configuration for this upstream.
    pub health_check_config: crate::types::health::UpstreamHealthCheckConfig,
    /// Weight for weighted load balancing algorithms (default 1).
    pub weight: u32,
    /// Optional mTLS credentials for this upstream.
    pub mtls: Option<Arc<MtlsCredentials>>,
    /// Priority for tiered failover. Lower values = higher priority.
    /// Default: 0 (highest priority).
    pub priority: u16,
    /// Use logical DNS mode (ToSocketAddrs at connect time, one backend).
    ///
    /// When false (default) and the URL contains a hostname, A/AAAA records
    /// are resolved via Hickory and each IP becomes a distinct backend
    /// (strict DNS mode).
    pub logical_dns: bool,
    /// Custom DNS servers for strict DNS resolution (empty = system resolver).
    pub dns_servers: Vec<std::net::IpAddr>,
    /// Optional connection timeout
    pub connection_timeout: Option<std::time::Duration>,
    /// Idle timeout for keepalive connections to this upstream.
    pub idle_timeout: std::time::Duration,
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
    /// Active health check configuration for this upstream.
    pub health_check_config: crate::types::health::UpstreamHealthCheckConfig,
    /// Weight for weighted load balancing algorithms (default 1).
    pub weight: u32,
    /// Optional mTLS credentials for the upstreams.
    pub mtls: Option<Arc<MtlsCredentials>>,
    /// Optional priority offset. When set, added to each DNS SRV priority
    /// to shift the entire block's priority tier. When None, DNS SRV
    /// priorities are used as-is.
    pub priority: Option<u16>,
    /// Optional connection timeout
    pub connection_timeout: Option<std::time::Duration>,
    /// Idle timeout for keepalive connections to this upstream.
    pub idle_timeout: std::time::Duration,
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
    /// Static upstreams resolve A/AAAA records via Hickory for hostnames
    /// (strict DNS mode), or pass through as-is for IP literals, Unix sockets,
    /// and logical DNS mode. SRV upstreams perform an SRV DNS lookup.
    #[inline]
    pub async fn resolve(
        &self,
        active_health_check_state: Option<HealthCheckStateMap>,
    ) -> Vec<Arc<UpstreamInner>> {
        match self {
            Upstream::Static(cfg) => {
                let needs_dns =
                    !cfg.logical_dns && cfg.unix_socket.is_none() && !is_ip_literal(&cfg.url);

                if needs_dns {
                    #[cfg(feature = "srv-lookup")]
                    {
                        super::strict_dns::resolve_strict_dns(cfg, active_health_check_state).await
                    }
                    #[cfg(not(feature = "srv-lookup"))]
                    {
                        // Fallback: no DNS resolution available, return as-is
                        vec![Arc::new(UpstreamInner {
                            proxy_to: cfg.url.clone(),
                            connect_to: None,
                            proxy_unix: cfg.unix_socket.clone(),
                            weight: cfg.weight,
                            mtls: cfg.mtls.clone(),
                            priority: cfg.priority,
                            connection_timeout: cfg.connection_timeout,
                            idle_timeout: cfg.idle_timeout,
                            dns_status: DnsResolutionStatus::NotApplicable,
                        })]
                    }
                } else {
                    let dns_status = if cfg.logical_dns {
                        DnsResolutionStatus::LogicalDns
                    } else {
                        DnsResolutionStatus::NotApplicable
                    };
                    vec![Arc::new(UpstreamInner {
                        proxy_to: cfg.url.clone(),
                        connect_to: None,
                        proxy_unix: cfg.unix_socket.clone(),
                        weight: cfg.weight,
                        mtls: cfg.mtls.clone(),
                        priority: cfg.priority,
                        connection_timeout: cfg.connection_timeout,
                        idle_timeout: cfg.idle_timeout,
                        dns_status,
                    })]
                }
            }
            #[cfg(feature = "srv-lookup")]
            Upstream::Srv(srv_data) => {
                super::srv::resolve_srv(srv_data, active_health_check_state).await
            }
        }
    }
}

/// Returns true if the URL host is an IP literal (IPv4 or IPv6).
#[inline]
fn is_ip_literal(url: &str) -> bool {
    let host = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .unwrap_or(url);
    let host = host.split('/').next().unwrap_or(host);
    let host = host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host);
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let host = host.to_lowercase();
    host.parse::<std::net::IpAddr>().is_ok()
}

/// mTLS credentials for a peer.
#[derive(Eq, PartialEq, Debug)]
pub struct MtlsCredentials {
    /// The client certificate chains, with each certificate encoded as DER.
    pub certs: Vec<rustls::pki_types::CertificateDer<'static>>,
    /// The private key, encoded as DER.
    pub key: rustls::pki_types::PrivateKeyDer<'static>,
}

impl Clone for MtlsCredentials {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            certs: self.certs.clone(),
            key: self.key.clone_key(),
        }
    }
}

impl std::hash::Hash for MtlsCredentials {
    #[inline]
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.certs.hash(state);
    }
}
