use std::io;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;

use arc_swap::ArcSwap;
use ferron_core::admin::ADMIN_METRICS;
use ferron_core::pipeline::Pipeline;
use ferron_core::providers::Provider;
use ferron_http::{HttpContext, HttpErrorContext, HttpFileContext};
use ferron_observability::{
    CompositeEventSink, Event, EventSink, LogEvent, LogLevel, ObservabilityContext,
};
use http::Request;
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::BodyExt;

use crate::config::ThreeStageResolver;
use crate::handler::request_handler;
use crate::server::tls_resolve::RadixTree;
use crate::server::HttpServerConfig;

pub const LOG_TARGET: &str = "ferron-http-server";
pub type ResponseBody = UnsyncBoxBody<bytes::Bytes, io::Error>;
pub type RequestHandlerFuture =
    Pin<Box<dyn std::future::Future<Output = Result<http::Response<ResponseBody>, io::Error>>>>;

/// Bundled shared state for the per-request handler closure.
/// Cloning a single `Arc<RequestHandlerState>` replaces cloning 6 individual
/// `Arc`s plus a `String` and a `CompositeEventSink`, reducing atomic
/// refcount contention at high RPS.
///
/// # Performance optimization
/// Connection-scoped radix tree lookups (observability, HTTP connection options)
/// are resolved once at connection setup and cached here, avoiding redundant
/// tree traversals on every HTTP request within the same connection.
pub struct RequestHandlerState {
    pub pipeline: Arc<Pipeline<HttpContext>>,
    pub file_pipeline: Arc<Pipeline<HttpFileContext>>,
    pub error_pipeline: Arc<Pipeline<HttpErrorContext>>,
    pub config_resolver: Arc<ThreeStageResolver>,
    /// Pre-resolved observability sinks for this connection's IP + SNI hostname.
    pub connection_observability: CompositeEventSink,
    /// Kept for virtual-hosting lookups when request Host header differs from SNI.
    pub observability_resolver: Arc<RadixTree<Vec<ObservabilityProviderEntry>>>,
    pub local_address: SocketAddr,
    pub remote_address: SocketAddr,
    /// Pre-normalized hostname from TLS SNI (available for per-request Host header overrides).
    pub hinted_hostname: Option<String>,
    pub encrypted: bool,
    pub http3_alt_svc: bool,
    pub https_port: Option<u16>,
}

// Type alias for the config ArcSwap
pub type ConfigArcSwap = Arc<ArcSwap<HttpServerConfig>>;

pub type ObservabilityProviderEntry = (
    Arc<dyn Provider<ObservabilityContext>>,
    Arc<ferron_core::config::ServerConfigurationBlock>,
);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HttpProtocols {
    pub http1: bool,
    pub http2: bool,
    pub http3: bool,
}

impl HttpProtocols {
    pub const fn empty() -> Self {
        Self {
            http1: false,
            http2: false,
            http3: false,
        }
    }

    pub const fn supports_http1(self) -> bool {
        self.http1
    }

    #[allow(dead_code)]
    pub const fn supports_http2(self) -> bool {
        self.http2
    }

    #[allow(dead_code)]
    pub const fn supports_http3(self) -> bool {
        self.http3
    }

    pub fn alpn_protocols(self) -> Vec<Vec<u8>> {
        let mut protocols = Vec::new();
        if self.http2 {
            protocols.push(b"h2".to_vec());
        }
        if self.http1 {
            protocols.push(b"http/1.1".to_vec());
            protocols.push(b"http/1.0".to_vec());
        }
        protocols
    }
}

impl Default for HttpProtocols {
    fn default() -> Self {
        Self {
            http1: true,
            http2: true,
            http3: false,
        }
    }
}

#[inline]
pub fn resolve_http_connection_options(
    resolver: &RadixTree<HttpConnectionOptions>,
    ip: IpAddr,
    hostname: Option<&str>,
) -> HttpConnectionOptions {
    let normalized_hostname = hostname.and_then(normalize_host_for_lookup);
    match normalized_hostname.as_deref() {
        Some(hostname) => resolver
            .lookup_ip_and_hostname(ip, hostname)
            .or_else(|| resolver.lookup_ip(ip)),
        None => resolver.lookup_ip(ip),
    }
    .or_else(|| resolver.root_data())
    .unwrap_or_default()
}

/// Initialize event sinks from provider entries (called at request/connection time).
#[inline]
fn initialize_sinks_from_providers(
    entries: &[ObservabilityProviderEntry],
) -> Vec<Arc<dyn EventSink>> {
    let mut sinks = Vec::with_capacity(entries.len());
    for (provider, log_config) in entries {
        let mut ctx = ObservabilityContext {
            log_config: log_config.clone(),
            sink: None,
        };
        if let Ok(()) = provider.execute(&mut ctx) {
            if let Some(sink) = ctx.sink {
                sinks.push(sink);
            }
        }
    }
    sinks
}

/// Helper to resolve root-level observability sinks (for pre-connection errors).
#[inline]
pub fn resolve_root_observability_sink(
    observability_resolver: &RadixTree<Vec<ObservabilityProviderEntry>>,
) -> CompositeEventSink {
    let sinks = observability_resolver
        .root_data()
        .map(|e| initialize_sinks_from_providers(&e))
        .unwrap_or_default();
    CompositeEventSink::new(sinks)
}

#[inline]
pub fn resolve_observability_sink(
    observability_resolver: &RadixTree<Vec<ObservabilityProviderEntry>>,
    ip: Option<IpAddr>,
    hostname: Option<&str>,
    fallback: &CompositeEventSink,
) -> CompositeEventSink {
    // Normalize hostname once to avoid redundant string operations
    let normalized_hostname = hostname.and_then(normalize_host_for_lookup);
    resolve_observability_sink_with_normalized_and_resolver(
        observability_resolver,
        ip,
        normalized_hostname.as_deref(),
        fallback,
    )
}

/// Optimized variant that accepts a pre-normalized hostname.
/// When the hostname matches the connection's SNI, returns the connection-level
/// sink directly without any radix tree lookup. Otherwise performs a fresh lookup.
#[inline]
fn resolve_observability_sink_with_normalized(
    observability_resolver: &RadixTree<Vec<ObservabilityProviderEntry>>,
    connection_observability: &CompositeEventSink,
    ip: IpAddr,
    hostname: Option<&str>,
) -> CompositeEventSink {
    // Fast path: if hostname matches SNI, return connection-level sink
    if hostname.is_none() {
        return connection_observability.clone();
    }

    // Slow path: hostname differs from SNI, need to lookup
    let entries = observability_resolver.lookup_ip_and_hostname(ip, hostname.unwrap());
    let sinks = entries
        .map(|e| initialize_sinks_from_providers(&e))
        .unwrap_or_default();
    if sinks.is_empty() {
        connection_observability.clone()
    } else {
        CompositeEventSink::new(sinks)
    }
}

/// Core implementation that performs the radix tree lookup with pre-normalized values.
#[inline]
fn resolve_observability_sink_with_normalized_and_resolver(
    observability_resolver: &RadixTree<Vec<ObservabilityProviderEntry>>,
    ip: Option<IpAddr>,
    hostname: Option<&str>,
    fallback: &CompositeEventSink,
) -> CompositeEventSink {
    let entries = match (ip, hostname) {
        (Some(ip), Some(hostname)) => observability_resolver.lookup_ip_and_hostname(ip, hostname),
        (Some(ip), None) => observability_resolver.lookup_ip(ip),
        (None, Some(hostname)) => observability_resolver.lookup_hostname(hostname),
        (None, None) => observability_resolver.root_data(),
    };

    let sinks = entries
        .map(|e| initialize_sinks_from_providers(&e))
        .unwrap_or_default();
    if sinks.is_empty() {
        fallback.clone()
    } else {
        CompositeEventSink::new(sinks)
    }
}

#[inline]
fn request_hostname_for_lookup<B>(
    request: &Request<B>,
    hinted_hostname: Option<&str>,
) -> Option<String> {
    request
        .headers()
        .get(http::header::HOST)
        .and_then(|value| value.to_str().ok())
        .and_then(normalize_host_for_lookup)
        .or_else(|| {
            request
                .uri()
                .authority()
                .map(|authority| authority.as_str())
                .and_then(normalize_host_for_lookup)
        })
        .or_else(|| hinted_hostname.map(std::borrow::ToOwned::to_owned))
}

#[inline]
pub fn emit_error(observability: &CompositeEventSink, message: impl Into<String>) {
    observability.emit(Event::Log(LogEvent {
        level: LogLevel::Error,
        message: message.into(),
        target: LOG_TARGET,
    }));
}

#[inline]
pub fn normalize_host_for_lookup(host: &str) -> Option<String> {
    let host = host.trim();
    if host.is_empty() {
        return None;
    }

    if let Some(rest) = host.strip_prefix('[') {
        let end = rest.find(']')?;
        return Some(rest[..end].to_ascii_lowercase());
    }

    let host_without_port = match host.rsplit_once(':') {
        Some((candidate, port))
            if !candidate.contains(':') && port.chars().all(|c| c.is_ascii_digit()) =>
        {
            candidate
        }
        _ => host,
    };
    let normalized = host_without_port.trim().trim_end_matches('.');
    if normalized.is_empty() {
        return None;
    }

    Some(normalized.to_ascii_lowercase())
}

#[inline]
pub fn build_request_handler(
    state: Arc<RequestHandlerState>,
) -> impl Fn(Request<vibeio_http::Incoming>) -> RequestHandlerFuture {
    move |request: Request<vibeio_http::Incoming>| {
        let state = Arc::clone(&state);
        Box::pin(async move {
            let hostname = request_hostname_for_lookup(&request, state.hinted_hostname.as_deref());
            // Reuse the connection-level observability sink; only re-resolve if the request
            // Host header differs from the SNI hostname (virtual hosting on the same connection).
            let request_observability = match hostname.as_deref() {
                Some(host) if state.hinted_hostname.as_deref() != Some(host) => {
                    resolve_observability_sink_with_normalized(
                        &state.observability_resolver,
                        &state.connection_observability,
                        state.local_address.ip(),
                        Some(host),
                    )
                }
                _ => state.connection_observability.clone(),
            };
            let (parts, body) = request.into_parts();
            let request = Request::from_parts(parts, body.boxed_unsync());
            request_handler(
                request,
                state.pipeline.clone(),
                state.file_pipeline.clone(),
                state.error_pipeline.clone(),
                state.config_resolver.clone(),
                state.local_address,
                state.remote_address,
                hostname,
                state.encrypted,
                state.http3_alt_svc,
                state.https_port,
                request_observability,
            )
            .await
        })
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Http2Settings {
    pub initial_window_size: Option<u32>,
    pub max_frame_size: Option<u32>,
    pub max_concurrent_streams: Option<u32>,
    pub max_header_list_size: Option<u32>,
    pub enable_connect_protocol: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HttpConnectionOptions {
    pub protocols: HttpProtocols,
    pub h1_enable_early_hints: bool,
    pub h2: Http2Settings,
    pub proxy_protocol_enabled: bool,
}

impl HttpConnectionOptions {
    pub fn alpn_protocols(&self) -> Vec<Vec<u8>> {
        self.protocols.alpn_protocols()
    }
}

/// RAII guard that decrements the active connection counter on drop.
pub struct ConnectionCountGuard;

impl ConnectionCountGuard {
    pub fn new() -> Self {
        ADMIN_METRICS
            .connections_active
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self
    }
}

impl Drop for ConnectionCountGuard {
    fn drop(&mut self) {
        ADMIN_METRICS
            .connections_active
            .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

#[derive(Debug)]
pub struct NoCertResolver;

impl rustls::server::ResolvesServerCert for NoCertResolver {
    #[inline]
    fn resolve(
        &self,
        _client_hello: rustls::server::ClientHello<'_>,
    ) -> Option<Arc<rustls::sign::CertifiedKey>> {
        None
    }
}
