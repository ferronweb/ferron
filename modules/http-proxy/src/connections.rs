//! Connection pool wrapper using connpool.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, RwLock};

use connpool::Pool;

use crate::send_request::SendRequestWrapper;
use crate::upstream::UpstreamInner;

/// Connection pool key type: (upstream, optional client IP for PROXY protocol).
pub type PoolKey = (UpstreamInner, Option<IpAddr>);

/// Connection pool manager for the reverse proxy.
pub struct ConnectionManager {
    connections: Arc<Pool<PoolKey, SendRequestWrapper>>,
    #[cfg(unix)]
    unix_connections: Arc<Pool<PoolKey, SendRequestWrapper>>,
    local_limits: RwLock<HashMap<UpstreamInner, usize>>,
}

impl ConnectionManager {
    pub fn with_global_limit(global_limit: usize) -> Self {
        Self {
            connections: Arc::new(Pool::new(global_limit)),
            #[cfg(unix)]
            unix_connections: Arc::new(Pool::new_unbounded()),
            local_limits: RwLock::new(HashMap::new()),
        }
    }

    /// Set a per-upstream local connection limit.
    pub fn set_local_limit(&self, upstream: &UpstreamInner, limit: usize) -> usize {
        let mut limits = self
            .local_limits
            .write()
            .expect("local_limits lock poisoned");
        if let Some(&idx) = limits.get(upstream) {
            return idx;
        }
        let idx = self.connections.set_local_limit(limit);
        #[cfg(unix)]
        let _ = self.unix_connections.set_local_limit(limit);
        limits.insert(upstream.clone(), idx);
        idx
    }

    /// Get the local limit index for an upstream.
    pub fn get_local_limit(&self, upstream: &UpstreamInner) -> Option<usize> {
        self.local_limits
            .read()
            .expect("local_limits lock poisoned")
            .get(upstream)
            .copied()
    }

    /// Pull a connection from the pool, waiting if necessary.
    #[allow(dead_code)]
    pub async fn pull(
        &self,
        upstream: &UpstreamInner,
        client_ip: Option<IpAddr>,
    ) -> connpool::Item<PoolKey, SendRequestWrapper> {
        let key = (upstream.clone(), client_ip);
        #[cfg(unix)]
        if upstream.proxy_unix.is_some() {
            return self.unix_connections.pull(key).await;
        }
        self.connections.pull(key).await
    }

    /// Pull a connection with a local limit applied, waiting if necessary.
    #[allow(dead_code)]
    pub async fn pull_with_local_limit(
        &self,
        upstream: &UpstreamInner,
        client_ip: Option<IpAddr>,
        local_limit_idx: Option<usize>,
    ) -> connpool::Item<PoolKey, SendRequestWrapper> {
        let key = (upstream.clone(), client_ip);
        #[cfg(unix)]
        if upstream.proxy_unix.is_some() {
            return self
                .unix_connections
                .pull_with_wait_local_limit(key, local_limit_idx)
                .await;
        }
        self.connections
            .pull_with_wait_local_limit(key, local_limit_idx)
            .await
    }

    pub fn connections(&self) -> &Arc<Pool<PoolKey, SendRequestWrapper>> {
        &self.connections
    }

    #[cfg(unix)]
    pub fn unix_connections(&self) -> &Arc<Pool<PoolKey, SendRequestWrapper>> {
        &self.unix_connections
    }
}
