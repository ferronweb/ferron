use std::collections::HashMap;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::Duration;

use hyper::header::HeaderName;
use tokio::sync::RwLock;

use super::{Connections, LoadBalancerAlgorithm, LoadBalancerAlgorithmInner, ProxyHeader, ProxyToKey, ReverseProxy};
use crate::util::TtlCache;

/// Builder for configuring and constructing a [`ReverseProxy`].
pub struct ReverseProxyBuilder<'a> {
  pub(super) connections: &'a mut Connections,
  #[allow(clippy::type_complexity)]
  pub(super) upstreams: Vec<(String, Option<String>, Option<usize>, Option<Duration>)>,
  pub(super) lb_algorithm: LoadBalancerAlgorithm,
  pub(super) lb_health_check_window: Duration,
  pub(super) lb_health_check_max_fails: u64,
  pub(super) lb_health_check: bool,
  pub(super) lb_retry_connection: bool,
  pub(super) proxy_no_verification: bool,
  pub(super) proxy_intercept_errors: bool,
  pub(super) proxy_http2_only: bool,
  pub(super) proxy_http2: bool,
  pub(super) proxy_keepalive: bool,
  pub(super) proxy_proxy_header: Option<ProxyHeader>,
  pub(super) proxy_request_header: Vec<(HeaderName, String)>,
  pub(super) proxy_request_header_replace: Vec<(HeaderName, String)>,
  pub(super) proxy_request_header_remove: Vec<HeaderName>,
}

impl<'a> ReverseProxyBuilder<'a> {
  /// Adds an upstream backend target.
  ///
  /// `proxy_to` is the backend URL (for example `http://127.0.0.1:8080`).
  /// `proxy_unix` can be used to target a Unix socket path.
  /// `local_limit` controls per-upstream connection limit.
  /// `keepalive_idle_timeout` sets pooled connection idle timeout.
  pub fn upstream(
    mut self,
    proxy_to: String,
    proxy_unix: Option<String>,
    local_limit: Option<usize>,
    keepalive_idle_timeout: Option<Duration>,
  ) -> Self {
    self
      .upstreams
      .push((proxy_to, proxy_unix, local_limit, keepalive_idle_timeout));
    self
  }

  /// Sets load balancing algorithm.
  pub fn lb_algorithm(mut self, algorithm: LoadBalancerAlgorithm) -> Self {
    self.lb_algorithm = algorithm;
    self
  }

  /// Sets health-check TTL window for failed backend counters.
  pub fn lb_health_check_window(mut self, window: Duration) -> Self {
    self.lb_health_check_window = window;
    self
  }

  /// Sets maximum consecutive failed checks before a backend is considered unhealthy.
  pub fn lb_health_check_max_fails(mut self, max_fails: u64) -> Self {
    self.lb_health_check_max_fails = max_fails;
    self
  }

  /// Enables or disables backend health checks.
  pub fn lb_health_check(mut self, enable: bool) -> Self {
    self.lb_health_check = enable;
    self
  }

  /// Disables certificate verification for upstream TLS connections.
  pub fn proxy_no_verification(mut self, no_verification: bool) -> Self {
    self.proxy_no_verification = no_verification;
    self
  }

  /// Intercepts upstream errors and converts them to proxy-generated responses.
  pub fn proxy_intercept_errors(mut self, intercept_errors: bool) -> Self {
    self.proxy_intercept_errors = intercept_errors;
    self
  }

  /// Enables retrying a different backend when connection setup fails.
  pub fn lb_retry_connection(mut self, retry: bool) -> Self {
    self.lb_retry_connection = retry;
    self
  }

  /// Forces HTTP/2-only upstream connections.
  pub fn proxy_http2_only(mut self, http2_only: bool) -> Self {
    self.proxy_http2_only = http2_only;
    self
  }

  /// Enables HTTP/2 support for upstream connections.
  pub fn proxy_http2(mut self, http2: bool) -> Self {
    self.proxy_http2 = http2;
    self
  }

  /// Enables connection pooling and keepalive reuse.
  pub fn proxy_keepalive(mut self, keepalive: bool) -> Self {
    self.proxy_keepalive = keepalive;
    self
  }

  /// Sets PROXY protocol header mode for upstream connections.
  pub fn proxy_proxy_header(mut self, proxy_header: Option<ProxyHeader>) -> Self {
    self.proxy_proxy_header = proxy_header;
    self
  }

  /// Adds a request header to upstream requests.
  pub fn proxy_request_header(mut self, header_name: HeaderName, header_value: String) -> Self {
    self.proxy_request_header.push((header_name, header_value));
    self
  }

  /// Replaces a request header on upstream requests.
  pub fn proxy_request_header_replace(mut self, header_name: HeaderName, header_value: String) -> Self {
    self.proxy_request_header_replace.push((header_name, header_value));
    self
  }

  /// Removes a request header from upstream requests.
  pub fn proxy_request_header_remove(mut self, header_name: HeaderName) -> Self {
    self.proxy_request_header_remove.push(header_name);
    self
  }

  /// Builds a [`ReverseProxy`] from the configured options.
  pub fn build(mut self) -> ReverseProxy {
    let connections = self.connections.connections.clone();
    #[cfg(unix)]
    let unix_connections = self.connections.unix_connections.clone();

    let proxy_to = self
      .upstreams
      .drain(..)
      .map(|(proxy_to, proxy_unix, local_limit, keepalive_idle_timeout)| {
        let is_unix_socket = proxy_unix.is_some();
        (
          proxy_to,
          proxy_unix,
          apply_local_limit(
            local_limit,
            is_unix_socket,
            &connections,
            #[cfg(unix)]
            &unix_connections,
          ),
          keepalive_idle_timeout,
        )
      })
      .collect::<Vec<ProxyToKey>>();

    let proxy_to = Arc::new(proxy_to);
    let load_balancer_algorithm = if let Some(algorithm) = self
      .connections
      .load_balancer_cache
      .get(&(self.lb_algorithm, proxy_to.clone()))
    {
      algorithm.clone()
    } else {
      let new_algorithm = Arc::new(build_load_balancer_algorithm(self.lb_algorithm));
      self
        .connections
        .load_balancer_cache
        .insert((self.lb_algorithm, proxy_to.clone()), new_algorithm.clone());
      new_algorithm
    };
    let failed_backends = if let Some(failed) = self.connections.failed_backend_cache.get(&(
      self.lb_health_check_window,
      self.lb_health_check_max_fails,
      proxy_to.clone(),
    )) {
      failed.clone()
    } else {
      let new_failed = Arc::new(RwLock::new(TtlCache::new(self.lb_health_check_window)));
      self.connections.failed_backend_cache.insert(
        (
          self.lb_health_check_window,
          self.lb_health_check_max_fails,
          proxy_to.clone(),
        ),
        new_failed.clone(),
      );
      new_failed
    };
    ReverseProxy {
      failed_backends,
      load_balancer_algorithm,
      proxy_to,
      health_check_max_fails: self.lb_health_check_max_fails,
      enable_health_check: self.lb_health_check,
      disable_certificate_verification: self.proxy_no_verification,
      proxy_intercept_errors: self.proxy_intercept_errors,
      retry_connection: self.lb_retry_connection,
      proxy_http2_only: self.proxy_http2_only,
      proxy_http2: self.proxy_http2,
      proxy_keepalive: self.proxy_keepalive,
      proxy_header: self.proxy_proxy_header,
      headers_to_add: Arc::new(self.proxy_request_header.drain(..).collect()),
      headers_to_replace: Arc::new(self.proxy_request_header_replace.drain(..).collect()),
      headers_to_remove: Arc::new(self.proxy_request_header_remove.drain(..).collect()),
      connections,
      #[cfg(unix)]
      unix_connections,
    }
  }
}

fn build_load_balancer_algorithm(algorithm: LoadBalancerAlgorithm) -> LoadBalancerAlgorithmInner {
  match algorithm {
    LoadBalancerAlgorithm::TwoRandomChoices => {
      LoadBalancerAlgorithmInner::TwoRandomChoices(Arc::new(RwLock::new(HashMap::new())))
    }
    LoadBalancerAlgorithm::LeastConnections => {
      LoadBalancerAlgorithmInner::LeastConnections(Arc::new(RwLock::new(HashMap::new())))
    }
    LoadBalancerAlgorithm::RoundRobin => LoadBalancerAlgorithmInner::RoundRobin(Arc::new(AtomicUsize::new(0))),
    LoadBalancerAlgorithm::Random => LoadBalancerAlgorithmInner::Random,
  }
}

fn apply_local_limit(
  local_limit: Option<usize>,
  is_unix_socket: bool,
  connections: &super::ConnectionPool,
  #[cfg(unix)] unix_connections: &super::ConnectionPool,
) -> Option<usize> {
  #[allow(clippy::bind_instead_of_map)]
  local_limit.and_then(|limit| {
    if is_unix_socket {
      #[cfg(unix)]
      {
        Some(unix_connections.set_local_limit(limit))
      }
      #[cfg(not(unix))]
      {
        None
      }
    } else {
      Some(connections.set_local_limit(limit))
    }
  })
}
