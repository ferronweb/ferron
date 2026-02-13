mod builder;
mod load_balancer;
mod proxy_client;
mod request_parts;
mod send_net_io;
mod send_request;

use std::collections::HashMap;
use std::error::Error;
use std::net::IpAddr;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use connpool::{Item, Pool};
use futures_util::FutureExt;
use http_body_util::combinators::BoxBody;
use hyper::header::{self, HeaderName};
use hyper::{Request, StatusCode, Uri};
#[cfg(feature = "runtime-monoio")]
use monoio::net::TcpStream;
#[cfg(all(feature = "runtime-monoio", unix))]
use monoio::net::UnixStream;
use rustls::client::WebPkiServerVerifier;
use rustls_pki_types::ServerName;
use rustls_platform_verifier::BuilderVerifierExt;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
#[cfg(feature = "runtime-tokio")]
use tokio::net::TcpStream;
#[cfg(all(feature = "runtime-tokio", unix))]
use tokio::net::UnixStream;
use tokio::sync::RwLock;
use tokio_rustls::TlsConnector;

use crate::config::ServerConfiguration;
use crate::http_proxy::send_request::SendRequestWrapper;
use crate::logging::ErrorLogger;
use crate::modules::{ModuleHandlers, ResponseData, SocketData};
use crate::observability::{Metric, MetricAttributeValue, MetricType, MetricValue, MetricsMultiSender};
use crate::util::{NoServerVerifier, TtlCache};

pub use self::builder::ReverseProxyBuilder;
#[cfg(feature = "runtime-monoio")]
use self::send_net_io::{SendTcpStreamPoll, SendTcpStreamPollDropGuard};
#[cfg(all(feature = "runtime-monoio", unix))]
use self::send_net_io::{SendUnixStreamPoll, SendUnixStreamPollDropGuard};
use self::{
  load_balancer::{determine_proxy_to, resolve_upstreams},
  proxy_client::{http_proxy, http_proxy_handshake},
  request_parts::construct_proxy_request_parts,
};

type ConnectionsTrackState = Arc<RwLock<HashMap<UpstreamInner, Arc<()>>>>;

enum LoadBalancerAlgorithmInner {
  Random,
  RoundRobin(Arc<AtomicUsize>),
  LeastConnections(ConnectionsTrackState),
  TwoRandomChoices(ConnectionsTrackState),
}

/// Backend selection strategy used when multiple upstreams are configured.
#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub enum LoadBalancerAlgorithm {
  /// Selects a backend randomly for each request.
  Random,
  /// Cycles through backends in order.
  RoundRobin,
  /// Selects the backend with the least active tracked connections.
  LeastConnections,
  /// Chooses two random backends and picks the less loaded one.
  TwoRandomChoices,
}

/// Proxy protocol version to prepend to upstream connections.
#[derive(Clone, Copy)]
pub enum ProxyHeader {
  /// HAProxy PROXY protocol v1.
  V1,
  /// HAProxy PROXY protocol v2.
  V2,
}

#[derive(Clone, Eq, PartialEq, Hash)]
struct UpstreamInner {
  proxy_to: String,
  proxy_unix: Option<String>,
}

#[derive(Clone)]
struct SrvUpstreamData {
  to: String,
  secondary_runtime_handle: tokio::runtime::Handle,
  dns_resolver: Arc<hickory_resolver::TokioResolver>,
}

impl PartialEq for SrvUpstreamData {
  fn eq(&self, other: &Self) -> bool {
    self.to == other.to
  }
}

impl Eq for SrvUpstreamData {}

impl std::hash::Hash for SrvUpstreamData {
  fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
    self.to.hash(state);
  }
}

#[derive(Clone, Eq, PartialEq, Hash)]
enum Upstream {
  Static(UpstreamInner),
  Srv(SrvUpstreamData),
}

impl Upstream {
  async fn resolve(
    &self,
    failed_backends: Arc<RwLock<TtlCache<UpstreamInner, u64>>>,
    health_check_max_fails: u64,
  ) -> Vec<UpstreamInner> {
    match self {
      Upstream::Static(inner) => vec![inner.clone()],
      Upstream::Srv(srv_data) => {
        let to = srv_data.to.clone();
        let resolver = srv_data.dns_resolver.clone();
        let failed_backends = failed_backends.clone();
        srv_data
          .secondary_runtime_handle
          .spawn(async move {
            let to_url = match Uri::from_str(&to) {
              Ok(uri) => uri,
              Err(_) => return vec![],
            };
            let to = match to_url.host() {
              Some(host) => host.to_string(),
              None => return vec![],
            };

            let srv_records = match resolver.srv_lookup(&to).await {
              Ok(records) => records,
              Err(_) => return vec![],
            };

            let failed_backends = failed_backends.read().await;
            let srv_upstreams = srv_records
              .into_iter()
              .filter_map(|record| {
                let mut to_url_parts = to_url.clone().into_parts();
                to_url_parts.authority = Some(format!("{}:{}", record.target(), record.port()).parse().ok()?);
                let upstream_inner = UpstreamInner {
                  proxy_to: Uri::from_parts(to_url_parts).ok()?.to_string(),
                  proxy_unix: None,
                };
                if failed_backends
                  .get(&upstream_inner)
                  .is_some_and(|fails| fails > health_check_max_fails)
                {
                  // Backend is unhealthy, skip it
                  None
                } else {
                  Some((upstream_inner, record.weight(), record.priority()))
                }
              })
              .collect::<Vec<_>>();
            let highest_priority = srv_upstreams
              .iter()
              .map(|(_, _, priority)| *priority)
              .min()
              .unwrap_or(0);
            let filtered_srv_upstreams = srv_upstreams
              .into_iter()
              .filter(|(_, _, priority)| *priority == highest_priority)
              .map(|(upstream, weight, _)| (upstream, weight))
              .collect::<Vec<_>>();
            let cumulative_weight: u64 = filtered_srv_upstreams.iter().map(|(_, weight)| *weight as u64).sum();
            let random_weight = if cumulative_weight == 0 {
              // Prevent empty range sampling panics
              0
            } else {
              rand::random_range(0..cumulative_weight)
            };
            for upstream in filtered_srv_upstreams {
              let weight = upstream.1;
              if random_weight <= weight as u64 {
                return vec![upstream.0];
              }
            }
            vec![]
          })
          .await
          .unwrap_or(vec![])
      }
    }
  }
}

type ProxyToKey = (Upstream, Option<usize>, Option<Duration>);
type ProxyToKeyInner = (UpstreamInner, Option<usize>, Option<Duration>);

type ConnectionPool = Arc<Pool<(UpstreamInner, Option<IpAddr>), SendRequestWrapper>>;
type ConnectionPoolItem = Item<(UpstreamInner, Option<IpAddr>), SendRequestWrapper>;

#[cfg(feature = "runtime-monoio")]
#[allow(unused)]
enum DropGuard {
  Tcp(SendTcpStreamPollDropGuard),
  #[cfg(unix)]
  Unix(SendUnixStreamPollDropGuard),
}

enum Connection {
  #[cfg(feature = "runtime-monoio")]
  Tcp(SendTcpStreamPoll),
  #[cfg(not(feature = "runtime-monoio"))]
  Tcp(TcpStream),
  #[cfg(all(feature = "runtime-monoio", unix))]
  Unix(SendUnixStreamPoll),
  #[cfg(all(not(feature = "runtime-monoio"), unix))]
  Unix(UnixStream),
}

#[cfg(feature = "runtime-monoio")]
impl Connection {
  unsafe fn get_drop_guard(&mut self) -> DropGuard {
    match self {
      Connection::Tcp(stream) => DropGuard::Tcp(stream.get_drop_guard()),
      #[cfg(unix)]
      Connection::Unix(stream) => DropGuard::Unix(stream.get_drop_guard()),
    }
  }
}

impl AsyncRead for Connection {
  fn poll_read(
    mut self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    buf: &mut tokio::io::ReadBuf,
  ) -> Poll<Result<(), std::io::Error>> {
    match &mut *self {
      Connection::Tcp(stream) => Pin::new(stream).poll_read(cx, buf),
      #[cfg(unix)]
      Connection::Unix(stream) => Pin::new(stream).poll_read(cx, buf),
    }
  }
}

impl AsyncWrite for Connection {
  fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize, std::io::Error>> {
    match &mut *self {
      Connection::Tcp(stream) => Pin::new(stream).poll_write(cx, buf),
      #[cfg(unix)]
      Connection::Unix(stream) => Pin::new(stream).poll_write(cx, buf),
    }
  }

  fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
    match &mut *self {
      Connection::Tcp(stream) => Pin::new(stream).poll_flush(cx),
      #[cfg(unix)]
      Connection::Unix(stream) => Pin::new(stream).poll_flush(cx),
    }
  }

  fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
    match &mut *self {
      Connection::Tcp(stream) => Pin::new(stream).poll_shutdown(cx),
      #[cfg(unix)]
      Connection::Unix(stream) => Pin::new(stream).poll_shutdown(cx),
    }
  }

  fn is_write_vectored(&self) -> bool {
    match self {
      Connection::Tcp(stream) => stream.is_write_vectored(),
      #[cfg(unix)]
      Connection::Unix(stream) => stream.is_write_vectored(),
    }
  }

  fn poll_write_vectored(
    mut self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    bufs: &[std::io::IoSlice<'_>],
  ) -> Poll<Result<usize, std::io::Error>> {
    match &mut *self {
      Connection::Tcp(stream) => Pin::new(stream).poll_write_vectored(cx, bufs),
      #[cfg(unix)]
      Connection::Unix(stream) => Pin::new(stream).poll_write_vectored(cx, bufs),
    }
  }
}

/// Connection pool for reverse proxy
pub struct Connections {
  #[allow(clippy::type_complexity)]
  load_balancer_cache: HashMap<
    (
      LoadBalancerAlgorithm,
      Arc<Vec<(Upstream, Option<usize>, Option<Duration>)>>,
    ),
    Arc<LoadBalancerAlgorithmInner>,
  >,
  #[allow(clippy::type_complexity)]
  failed_backend_cache: HashMap<
    (Duration, u64, Arc<Vec<(Upstream, Option<usize>, Option<Duration>)>>),
    Arc<RwLock<TtlCache<UpstreamInner, u64>>>,
  >,
  connections: ConnectionPool,
  #[cfg(unix)]
  unix_connections: ConnectionPool,
}

impl Connections {
  /// Creates a connection pool without a global connection limit.
  pub fn new() -> Self {
    Self {
      load_balancer_cache: HashMap::new(),
      failed_backend_cache: HashMap::new(),
      connections: Arc::new(Pool::new_unbounded()),
      #[cfg(unix)]
      unix_connections: Arc::new(Pool::new_unbounded()),
    }
  }

  /// Creates a connection pool with a global TCP connection limit.
  ///
  /// Unix socket connections remain unbounded.
  pub fn with_global_limit(global_limit: usize) -> Self {
    Self {
      load_balancer_cache: HashMap::new(),
      failed_backend_cache: HashMap::new(),
      connections: Arc::new(Pool::new(global_limit)),
      #[cfg(unix)]
      unix_connections: Arc::new(Pool::new_unbounded()),
    }
  }

  /// Starts a reverse proxy builder using this connection pool.
  pub fn get_builder<'a>(&'a mut self) -> ReverseProxyBuilder<'a> {
    ReverseProxyBuilder {
      connections: self,
      upstreams: Vec::new(),
      lb_algorithm: LoadBalancerAlgorithm::TwoRandomChoices,
      lb_health_check_window: Duration::from_millis(5000),
      lb_health_check_max_fails: 3,
      lb_health_check: false,
      proxy_no_verification: false,
      proxy_intercept_errors: false,
      lb_retry_connection: true,
      proxy_http2_only: false,
      proxy_http2: false,
      proxy_keepalive: true,
      proxy_proxy_header: None,
      proxy_request_header: Vec::new(),
      proxy_request_header_replace: Vec::new(),
      proxy_request_header_remove: Vec::new(),
    }
  }
}

impl Default for Connections {
  fn default() -> Self {
    Self::new()
  }
}

/// A reverse proxy
pub struct ReverseProxy {
  #[allow(clippy::type_complexity)]
  failed_backends: Arc<RwLock<TtlCache<UpstreamInner, u64>>>,
  load_balancer_algorithm: Arc<LoadBalancerAlgorithmInner>,
  proxy_to: Arc<Vec<ProxyToKey>>,
  health_check_max_fails: u64,
  enable_health_check: bool,
  disable_certificate_verification: bool,
  proxy_intercept_errors: bool,
  retry_connection: bool,
  proxy_http2_only: bool,
  proxy_http2: bool,
  proxy_keepalive: bool,
  proxy_header: Option<ProxyHeader>,
  headers_to_add: Arc<Vec<(HeaderName, String)>>,
  headers_to_replace: Arc<Vec<(HeaderName, String)>>,
  headers_to_remove: Arc<Vec<HeaderName>>,
  connections: ConnectionPool,
  #[cfg(unix)]
  unix_connections: ConnectionPool,
}

impl ReverseProxy {
  /// Creates a request handler instance with shared proxy state.
  pub fn get_handler(&self) -> ReverseProxyHandler {
    ReverseProxyHandler {
      failed_backends: self.failed_backends.clone(),
      load_balancer_algorithm: self.load_balancer_algorithm.clone(),
      proxy_to: self.proxy_to.clone(),
      health_check_max_fails: self.health_check_max_fails,
      selected_backends_metrics: None,
      unhealthy_backends_metrics: None,
      connection_reused: false,
      enable_health_check: self.enable_health_check,
      disable_certificate_verification: self.disable_certificate_verification,
      proxy_intercept_errors: self.proxy_intercept_errors,
      retry_connection: self.retry_connection,
      proxy_http2_only: self.proxy_http2_only,
      proxy_http2: self.proxy_http2,
      proxy_keepalive: self.proxy_keepalive,
      proxy_header: self.proxy_header,
      headers_to_add: self.headers_to_add.clone(),
      headers_to_replace: self.headers_to_replace.clone(),
      headers_to_remove: self.headers_to_remove.clone(),
      connections: self.connections.clone(),
      #[cfg(unix)]
      unix_connections: self.unix_connections.clone(),
    }
  }
}

/// Handlers for the reverse proxy module
pub struct ReverseProxyHandler {
  #[allow(clippy::type_complexity)]
  failed_backends: Arc<RwLock<TtlCache<UpstreamInner, u64>>>,
  load_balancer_algorithm: Arc<LoadBalancerAlgorithmInner>,
  proxy_to: Arc<Vec<ProxyToKey>>,
  health_check_max_fails: u64,
  selected_backends_metrics: Option<Vec<UpstreamInner>>,
  unhealthy_backends_metrics: Option<Vec<UpstreamInner>>,
  connection_reused: bool,
  enable_health_check: bool,
  disable_certificate_verification: bool,
  proxy_intercept_errors: bool,
  retry_connection: bool,
  proxy_http2_only: bool,
  proxy_http2: bool,
  proxy_keepalive: bool,
  proxy_header: Option<ProxyHeader>,
  headers_to_add: Arc<Vec<(HeaderName, String)>>,
  headers_to_replace: Arc<Vec<(HeaderName, String)>>,
  headers_to_remove: Arc<Vec<HeaderName>>,
  connections: ConnectionPool,
  #[cfg(unix)]
  unix_connections: ConnectionPool,
}

impl ReverseProxyHandler {
  #[inline]
  fn status_response(status_code: StatusCode) -> ResponseData {
    ResponseData {
      request: None,
      response: None,
      response_status: Some(status_code),
      response_headers: None,
      new_remote_address: None,
    }
  }

  async fn mark_backend_failure(&mut self, upstream: &UpstreamInner) {
    if !self.enable_health_check {
      return;
    }
    if let Some(unhealthy_backends_metrics) = self.unhealthy_backends_metrics.as_mut() {
      unhealthy_backends_metrics.push(upstream.clone());
    }
    let mut failed_backends_write = self.failed_backends.write().await;
    let failed_attempts = failed_backends_write.get(upstream);
    failed_backends_write.insert(upstream.clone(), failed_attempts.map_or(1, |x| x + 1));
  }

  async fn retry_or_respond(
    &self,
    error_logger: &ErrorLogger,
    err: &dyn std::fmt::Display,
    retry_connection: bool,
    has_more_backends: bool,
    status_code: StatusCode,
    log_prefix: &str,
  ) -> Option<ResponseData> {
    if retry_connection && has_more_backends {
      error_logger
        .log(&format!("Failed to connect to backend, trying another backend: {err}"))
        .await;
      None
    } else {
      error_logger.log(&format!("{log_prefix}: {err}")).await;
      Some(Self::status_response(status_code))
    }
  }

  #[inline]
  fn io_error_status(err: &std::io::Error) -> (StatusCode, &'static str) {
    match err.kind() {
      std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound | std::io::ErrorKind::HostUnreachable => {
        (StatusCode::SERVICE_UNAVAILABLE, "Service unavailable")
      }
      std::io::ErrorKind::TimedOut => (StatusCode::GATEWAY_TIMEOUT, "Gateway timeout"),
      _ => (StatusCode::BAD_GATEWAY, "Bad gateway"),
    }
  }
}

#[async_trait(?Send)]
impl ModuleHandlers for ReverseProxyHandler {
  /// Handles incoming HTTP requests and proxies them to the configured backend server(s)
  ///
  /// This handler:
  /// 1. Determines which backend server to proxy to (supports load balancing)
  /// 2. Transforms the request by:
  ///    - Converting the URL to match the backend format
  ///    - Setting appropriate headers (Host, X-Forwarded-*)
  /// 3. Establishes a connection to the backend (HTTP or HTTPS)
  /// 4. Forwards the request and returns the response
  ///
  /// The handler supports:
  /// - Load balancing across multiple backends
  /// - Connection pooling/reuse
  /// - Health checking (marking failed backends)
  /// - TLS/SSL for secure connections
  /// - HTTP protocol upgrades (e.g., WebSockets)
  async fn request_handler(
    &mut self,
    request: Request<BoxBody<Bytes, std::io::Error>>,
    config: &ServerConfiguration,
    socket_data: &SocketData,
    error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    let enable_health_check = self.enable_health_check;
    let health_check_max_fails = self.health_check_max_fails;
    let disable_certificate_verification = self.disable_certificate_verification;
    let proxy_intercept_errors = self.proxy_intercept_errors;
    if self.proxy_to.is_empty() {
      // No upstreams configured...
      return Ok(ResponseData {
        request: Some(request),
        response: None,
        response_status: None,
        response_headers: None,
        new_remote_address: None,
      });
    }
    let mut proxy_to_vector = resolve_upstreams(
      &self.proxy_to,
      self.failed_backends.clone(),
      self.health_check_max_fails,
    )
    .await;
    let load_balancer_algorithm = self.load_balancer_algorithm.clone();
    let connection_track = match &*load_balancer_algorithm {
      LoadBalancerAlgorithmInner::LeastConnections(connection_track) => Some(connection_track),
      LoadBalancerAlgorithmInner::TwoRandomChoices(connection_track) => Some(connection_track),
      _ => None,
    };
    let retry_connection = self.retry_connection;
    let (request_parts, request_body) = request.into_parts();
    let mut request_parts = Some(request_parts);

    loop {
      if let Some((upstream, local_limit_index, keepalive_idle_timeout)) = determine_proxy_to(
        &mut proxy_to_vector,
        &self.failed_backends,
        enable_health_check,
        health_check_max_fails,
        &load_balancer_algorithm,
      )
      .await
      {
        if let Some(selected_backends_metrics) = self.selected_backends_metrics.as_mut() {
          selected_backends_metrics.push(upstream.clone());
        }
        let UpstreamInner { proxy_to, proxy_unix } = &upstream;
        let proxy_request_url = proxy_to.parse::<hyper::Uri>()?;
        let scheme_str = proxy_request_url.scheme_str();
        let mut encrypted = false;

        match scheme_str {
          Some("http") => {
            encrypted = false;
          }
          Some("https") => {
            encrypted = true;
          }
          _ => Err(anyhow::anyhow!("Only HTTP and HTTPS reverse proxy URLs are supported."))?,
        };

        let host = match proxy_request_url.host() {
          Some(host) => host,
          None => Err(anyhow::anyhow!("The reverse proxy URL doesn't include the host"))?,
        };

        let port = proxy_request_url.port_u16().unwrap_or(match scheme_str {
          Some("http") => 80,
          Some("https") => 443,
          _ => 80,
        });

        let addr = format!("{host}:{port}");

        let request_parts_option = if proxy_to_vector.is_empty() {
          request_parts.take()
        } else {
          request_parts.clone()
        };
        let request_parts = request_parts_option.ok_or(anyhow::anyhow!("Request parts not found"))?;
        let proxy_request_parts = construct_proxy_request_parts(
          request_parts,
          config,
          socket_data,
          &proxy_request_url,
          &self.headers_to_add,
          &self.headers_to_replace,
          &self.headers_to_remove,
        )?;

        let tracked_connection = if let Some(connection_track) = connection_track {
          let connection_track_read = connection_track.read().await;
          Some(if let Some(connection_count) = connection_track_read.get(&upstream) {
            connection_count.clone()
          } else {
            let tracked_connection = Arc::new(());
            drop(connection_track_read);
            connection_track
              .write()
              .await
              .insert(upstream.clone(), tracked_connection.clone());
            tracked_connection
          })
        } else {
          None
        };

        let proxy_header = self.proxy_header;

        let is_http_upgrade = proxy_request_parts.headers.contains_key(header::UPGRADE);
        let enable_http2_only_config = self.proxy_http2_only;
        let enable_http2_config = self.proxy_http2;

        let enable_keepalive =
          (enable_http2_only_config || !enable_http2_config || !is_http_upgrade) && self.proxy_keepalive;
        let connection_pool_item = {
          #[cfg(unix)]
          let connections = if proxy_unix.is_some() {
            &self.unix_connections
          } else {
            &self.connections
          };
          #[cfg(not(unix))]
          let connections = &self.connections;
          let sender;
          let mut send_request_items = Vec::new();
          let proxy_client_ip = match proxy_header {
            Some(ProxyHeader::V1) | Some(ProxyHeader::V2) => Some(socket_data.remote_addr.ip().to_canonical()),
            _ => None,
          };
          loop {
            let mut send_request_item = if send_request_items.is_empty() {
              connections
                .pull_with_wait_local_limit((upstream.clone(), proxy_client_ip), local_limit_index)
                .await
            } else if let Poll::Ready(send_request_item_option) = connections
              .pull_with_wait_local_limit((upstream.clone(), proxy_client_ip), local_limit_index)
              .boxed_local()
              .poll_unpin(&mut Context::from_waker(Waker::noop()))
            {
              send_request_item_option
            } else {
              let send_request_items_taken = send_request_items;
              send_request_items = Vec::new();
              let fetch_nonready_send_request_fut = async {
                let result = futures_util::future::select_ok(send_request_items_taken).await;
                if let Ok((item, send_request_items_smaller)) = result {
                  send_request_items = send_request_items_smaller;
                  item
                } else {
                  futures_util::future::pending().await
                }
              };
              crate::runtime::select! {
                item = connections
                  .pull_with_wait_local_limit((upstream.clone(), proxy_client_ip), local_limit_index)
                => {
                  item
                },
                item = fetch_nonready_send_request_fut => {
                  item
                }
              }
            };
            if let Some(send_request) = send_request_item.inner_mut() {
              match send_request.get(keepalive_idle_timeout) {
                (Some(send_request), true) => {
                  // Connection ready, send a request to it
                  send_request_items.clear();
                  self.connection_reused = true;
                  let _ = send_request_item.inner_mut().take();
                  let proxy_request = Request::from_parts(proxy_request_parts, request_body);
                  let result = http_proxy(
                    send_request,
                    send_request_item,
                    proxy_request,
                    error_logger,
                    proxy_intercept_errors,
                    tracked_connection,
                    true,
                  )
                  .await;
                  return result;
                }
                (None, true) => {
                  // Connection not ready
                  send_request_items.push(Box::pin(async move {
                    let inner_item = send_request_item.inner_mut();
                    if let Some(inner_item_2) = inner_item {
                      if !inner_item_2.wait_ready(keepalive_idle_timeout).await {
                        // Connection closed or timed out
                        inner_item.take();
                        return Err(());
                      }
                      let _ = inner_item;
                      Ok(send_request_item)
                    } else {
                      Err(())
                    }
                  }));
                  continue;
                }
                (_, false) => {
                  // Connection closed
                  let _ = send_request_item.inner_mut().take();
                  continue;
                }
              }
            }
            send_request_items.clear();
            sender = send_request_item;
            break;
          }
          sender
        };

        let stream = if let Some(proxy_unix_str) = &proxy_unix {
          #[cfg(not(unix))]
          {
            let _ = proxy_unix_str; // Discard the variable to avoid unused variable warning
            Err(anyhow::anyhow!("Unix sockets are not supported on this platform"))?
          }

          #[cfg(unix)]
          {
            let stream = match UnixStream::connect(proxy_unix_str).await {
              Ok(stream) => stream,
              Err(err) => {
                self.mark_backend_failure(&upstream).await;
                let (status_code, log_prefix) = Self::io_error_status(&err);
                if let Some(response) = self
                  .retry_or_respond(
                    error_logger,
                    &err,
                    retry_connection,
                    !proxy_to_vector.is_empty(),
                    status_code,
                    log_prefix,
                  )
                  .await
                {
                  return Ok(response);
                }
                continue;
              }
            };

            #[cfg(feature = "runtime-monoio")]
            let stream = match SendUnixStreamPoll::new_comp_io(stream) {
              Ok(stream) => stream,
              Err(err) => {
                self.mark_backend_failure(&upstream).await;
                if let Some(response) = self
                  .retry_or_respond(
                    error_logger,
                    &err,
                    retry_connection,
                    !proxy_to_vector.is_empty(),
                    StatusCode::BAD_GATEWAY,
                    "Bad gateway",
                  )
                  .await
                {
                  return Ok(response);
                }
                continue;
              }
            };

            Connection::Unix(stream)
          }
        } else {
          let stream = match TcpStream::connect(&addr).await {
            Ok(stream) => stream,
            Err(err) => {
              self.mark_backend_failure(&upstream).await;
              let (status_code, log_prefix) = Self::io_error_status(&err);
              if let Some(response) = self
                .retry_or_respond(
                  error_logger,
                  &err,
                  retry_connection,
                  !proxy_to_vector.is_empty(),
                  status_code,
                  log_prefix,
                )
                .await
              {
                return Ok(response);
              }
              continue;
            }
          };

          if let Err(err) = stream.set_nodelay(true) {
            self.mark_backend_failure(&upstream).await;
            if let Some(response) = self
              .retry_or_respond(
                error_logger,
                &err,
                retry_connection,
                !proxy_to_vector.is_empty(),
                StatusCode::BAD_GATEWAY,
                "Bad gateway",
              )
              .await
            {
              return Ok(response);
            }
            continue;
          };

          #[cfg(feature = "runtime-monoio")]
          let stream = match SendTcpStreamPoll::new_comp_io(stream) {
            Ok(stream) => stream,
            Err(err) => {
              self.mark_backend_failure(&upstream).await;
              if let Some(response) = self
                .retry_or_respond(
                  error_logger,
                  &err,
                  retry_connection,
                  !proxy_to_vector.is_empty(),
                  StatusCode::BAD_GATEWAY,
                  "Bad gateway",
                )
                .await
              {
                return Ok(response);
              }
              continue;
            }
          };

          Connection::Tcp(stream)
        };

        let proxy_header_to_write = match proxy_header {
          Some(ProxyHeader::V1) => {
            let is_ipv4 = socket_data.local_addr.ip().to_canonical().is_ipv4()
              && socket_data.remote_addr.ip().to_canonical().is_ipv4();
            let local_addr = if is_ipv4 {
              match socket_data.local_addr.ip().to_canonical() {
                IpAddr::V4(ip) => ip.to_string(),
                IpAddr::V6(ip) => ip
                  .to_ipv4_mapped()
                  .ok_or(anyhow::anyhow!("Connection IP address type mismatch"))?
                  .to_string(),
              }
            } else {
              match socket_data.local_addr.ip().to_canonical() {
                IpAddr::V4(ip) => ip
                  .to_ipv6_mapped()
                  .segments()
                  .iter()
                  .map(|seg| format!("{:04x}", seg))
                  .collect::<Vec<_>>()
                  .join(":"),
                IpAddr::V6(ip) => ip
                  .segments()
                  .iter()
                  .map(|seg| format!("{:04x}", seg))
                  .collect::<Vec<_>>()
                  .join(":"),
              }
            };
            let remote_addr = if is_ipv4 {
              match socket_data.remote_addr.ip().to_canonical() {
                IpAddr::V4(ip) => ip.to_string(),
                IpAddr::V6(ip) => ip
                  .to_ipv4_mapped()
                  .ok_or(anyhow::anyhow!("Connection IP address type mismatch"))?
                  .to_string(),
              }
            } else {
              match socket_data.remote_addr.ip().to_canonical() {
                IpAddr::V4(ip) => ip
                  .to_ipv6_mapped()
                  .segments()
                  .iter()
                  .map(|seg| format!("{:04x}", seg))
                  .collect::<Vec<_>>()
                  .join(":"),
                IpAddr::V6(ip) => ip
                  .segments()
                  .iter()
                  .map(|seg| format!("{:04x}", seg))
                  .collect::<Vec<_>>()
                  .join(":"),
              }
            };
            let local_port = socket_data.local_addr.port();
            let remote_port = socket_data.remote_addr.port();
            let header = format!(
              "PROXY {} {} {} {} {}\r\n",
              if is_ipv4 { "TCP4" } else { "TCP6" },
              remote_addr,
              local_addr,
              remote_port,
              local_port,
            );
            Some(header.into_bytes())
          }
          Some(ProxyHeader::V2) => {
            let is_ipv4 = socket_data.local_addr.ip().to_canonical().is_ipv4()
              && socket_data.remote_addr.ip().to_canonical().is_ipv4();
            let addresses = if is_ipv4 {
              ppp::v2::Addresses::IPv4(ppp::v2::IPv4::new(
                match socket_data.remote_addr.ip().to_canonical() {
                  IpAddr::V4(ip) => ip,
                  IpAddr::V6(ip) => ip
                    .to_ipv4_mapped()
                    .ok_or(anyhow::anyhow!("Connection IP address type mismatch"))?,
                },
                match socket_data.local_addr.ip().to_canonical() {
                  IpAddr::V4(ip) => ip,
                  IpAddr::V6(ip) => ip
                    .to_ipv4_mapped()
                    .ok_or(anyhow::anyhow!("Connection IP address type mismatch"))?,
                },
                socket_data.remote_addr.port(),
                socket_data.local_addr.port(),
              ))
            } else {
              ppp::v2::Addresses::IPv6(ppp::v2::IPv6::new(
                match socket_data.remote_addr.ip().to_canonical() {
                  IpAddr::V4(ip) => ip.to_ipv6_mapped(),
                  IpAddr::V6(ip) => ip,
                },
                match socket_data.local_addr.ip().to_canonical() {
                  IpAddr::V4(ip) => ip.to_ipv6_mapped(),
                  IpAddr::V6(ip) => ip,
                },
                socket_data.remote_addr.port(),
                socket_data.local_addr.port(),
              ))
            };
            let header_builder = ppp::v2::Builder::with_addresses(
              ppp::v2::Version::Two | ppp::v2::Command::Proxy,
              ppp::v2::Protocol::Stream,
              addresses,
            );
            Some(header_builder.build()?)
          }
          _ => None,
        };

        let mut stream = stream; // Make the stream a mutable variable (to be able to write PROXY protocol header to it).

        if let Some(proxy_header_to_write) = proxy_header_to_write {
          if let Err(err) = stream.write_all(&proxy_header_to_write).await {
            self.mark_backend_failure(&upstream).await;
            if let Some(response) = self
              .retry_or_respond(
                error_logger,
                &err,
                retry_connection,
                !proxy_to_vector.is_empty(),
                StatusCode::BAD_GATEWAY,
                "Bad gateway",
              )
              .await
            {
              return Ok(response);
            }
            continue;
          }
        }

        // Safety: the drop guard is dropped when the connection future is completed,
        // and after the underlying connection is moved across threads,
        // see the "http_proxy_handshake" function.
        #[cfg(feature = "runtime-monoio")]
        let drop_guard = unsafe { stream.get_drop_guard() };

        let sender = if !encrypted {
          let sender = match http_proxy_handshake(
            stream,
            enable_http2_only_config,
            #[cfg(feature = "runtime-monoio")]
            drop_guard,
          )
          .await
          {
            Ok(sender) => sender,
            Err(err) => {
              self.mark_backend_failure(&upstream).await;
              if let Some(response) = self
                .retry_or_respond(
                  error_logger,
                  &err,
                  retry_connection,
                  !proxy_to_vector.is_empty(),
                  StatusCode::BAD_GATEWAY,
                  "Bad gateway",
                )
                .await
              {
                return Ok(response);
              }
              continue;
            }
          };

          sender
        } else {
          let enable_http2_config = enable_http2_only_config || (enable_http2_config && !is_http_upgrade);
          let mut tls_client_config = (if disable_certificate_verification {
            rustls::ClientConfig::builder()
              .dangerous()
              .with_custom_certificate_verifier(Arc::new(NoServerVerifier::new()))
          } else if let Ok(client_config) = BuilderVerifierExt::with_platform_verifier(rustls::ClientConfig::builder())
          {
            client_config
          } else {
            rustls::ClientConfig::builder().with_webpki_verifier(
              WebPkiServerVerifier::builder(Arc::new(rustls::RootCertStore {
                roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
              }))
              .build()?,
            )
          })
          .with_no_client_auth();
          if enable_http2_only_config {
            tls_client_config.alpn_protocols = vec![b"h2".to_vec()];
          } else if enable_http2_config {
            tls_client_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec(), b"http/1.0".to_vec()];
          } else {
            tls_client_config.alpn_protocols = vec![b"http/1.1".to_vec(), b"http/1.0".to_vec()];
          }
          let connector = TlsConnector::from(Arc::new(tls_client_config));
          let domain = ServerName::try_from(host)?.to_owned();

          let tls_stream = match connector.connect(domain, stream).await {
            Ok(stream) => stream,
            Err(err) => {
              self.mark_backend_failure(&upstream).await;
              if let Some(response) = self
                .retry_or_respond(
                  error_logger,
                  &err,
                  retry_connection,
                  !proxy_to_vector.is_empty(),
                  StatusCode::BAD_GATEWAY,
                  "Bad gateway",
                )
                .await
              {
                return Ok(response);
              }
              continue;
            }
          };

          // Enable HTTP/2 when the ALPN protocol is "h2"
          let enable_http2 = enable_http2_config && tls_stream.get_ref().1.alpn_protocol() == Some(b"h2");

          let sender = match http_proxy_handshake(
            tls_stream,
            enable_http2,
            #[cfg(feature = "runtime-monoio")]
            drop_guard,
          )
          .await
          {
            Ok(sender) => sender,
            Err(err) => {
              self.mark_backend_failure(&upstream).await;
              if let Some(response) = self
                .retry_or_respond(
                  error_logger,
                  &err,
                  retry_connection,
                  !proxy_to_vector.is_empty(),
                  StatusCode::BAD_GATEWAY,
                  "Bad gateway",
                )
                .await
              {
                return Ok(response);
              }
              continue;
            }
          };

          sender
        };

        let proxy_request = Request::from_parts(proxy_request_parts, request_body);

        return http_proxy(
          sender,
          connection_pool_item,
          proxy_request,
          error_logger,
          proxy_intercept_errors,
          tracked_connection,
          enable_keepalive,
        )
        .await;
      } else {
        let request_parts = request_parts.ok_or(anyhow::anyhow!("Request parts are missing"))?;
        error_logger.log("No upstreams available").await;
        return Ok(ResponseData {
          request: Some(Request::from_parts(request_parts, request_body)),
          response: None,
          response_status: Some(StatusCode::SERVICE_UNAVAILABLE), // No upstreams available
          response_headers: None,
          new_remote_address: None,
        });
      }
    }
  }

  async fn metric_data_before_handler(
    &mut self,
    _request: &Request<BoxBody<Bytes, std::io::Error>>,
    _socket_data: &SocketData,
    _metrics_sender: &MetricsMultiSender,
  ) {
    self.selected_backends_metrics = Some(Vec::new());
    self.unhealthy_backends_metrics = Some(Vec::new());
  }

  async fn metric_data_after_handler(&mut self, metrics_sender: &MetricsMultiSender) {
    if let Some(selected_backends_metrics) = self.selected_backends_metrics.take() {
      for selected_backend in selected_backends_metrics {
        let mut attributes = Vec::new();
        attributes.push((
          "ferron.proxy.backend_url",
          MetricAttributeValue::String(selected_backend.proxy_to),
        ));
        if let Some(backend_unix) = selected_backend.proxy_unix {
          attributes.push((
            "ferron.proxy.backend_unix_path",
            MetricAttributeValue::String(backend_unix),
          ));
        }
        metrics_sender
          .send(Metric::new(
            "ferron.proxy.backends.selected",
            attributes,
            MetricType::Counter,
            MetricValue::U64(1),
            Some("{backend}"),
            Some("Number of times a backend server was selected."),
          ))
          .await;
      }
    }
    if let Some(unhealthy_backends_metrics) = self.unhealthy_backends_metrics.take() {
      for unhealthy_backend in unhealthy_backends_metrics {
        let mut attributes = Vec::new();
        attributes.push((
          "ferron.proxy.backend_url",
          MetricAttributeValue::String(unhealthy_backend.proxy_to),
        ));
        if let Some(backend_unix) = unhealthy_backend.proxy_unix {
          attributes.push((
            "ferron.proxy.backend_unix_path",
            MetricAttributeValue::String(backend_unix),
          ));
        }
        metrics_sender
          .send(Metric::new(
            "ferron.proxy.backends.unhealthy",
            attributes,
            MetricType::Counter,
            MetricValue::U64(1),
            Some("{backend}"),
            Some("Number of health check failures for a backend server."),
          ))
          .await;
      }
    }
    metrics_sender
      .send(Metric::new(
        "ferron.proxy.requests",
        vec![(
          "ferron.proxy.connection_reused",
          MetricAttributeValue::Bool(self.connection_reused),
        )],
        MetricType::Counter,
        MetricValue::U64(1),
        Some("{request}"),
        Some("Number of reverse proxy requests."),
      ))
      .await;
  }
}
