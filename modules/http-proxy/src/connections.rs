//! Connection pool wrapper using connpool.

use std::collections::{hash_map::DefaultHasher, HashMap};
use std::hash::{Hash, Hasher};
use std::net::IpAddr;
use std::sync::{Arc, RwLock};

use connpool::Pool;

use crate::send_request::SendRequestWrapper;
use crate::upstream::UpstreamInner;

/// Connection pool key type: (upstream, optional client IP for PROXY protocol).
pub type PoolKey = (UpstreamInner, Option<IpAddr>);

/// Connection pool manager for the reverse proxy.
pub struct ConnectionManager {
    connections: Vec<Arc<Pool<PoolKey, SendRequestWrapper>>>,
    #[cfg(unix)]
    unix_connections: Arc<Pool<PoolKey, SendRequestWrapper>>,
    local_limits: RwLock<HashMap<UpstreamInner, usize>>,
}

impl ConnectionManager {
    pub fn with_global_limit(global_limit: usize) -> Self {
        Self::with_global_limit_and_shards(global_limit, std::cmp::max(1, num_cpus::get()))
    }

    /// Like with_global_limit but allows overriding the number of shards (useful for tests/benches).
    pub fn with_global_limit_and_shards(global_limit: usize, shards: usize) -> Self {
        let shards = std::cmp::max(1, shards);
        let per_shard = if shards > 0 {
            global_limit.div_ceil(shards)
        } else {
            global_limit
        };
        let mut conns = Vec::with_capacity(shards);
        for _ in 0..shards {
            conns.push(Arc::new(Pool::new(per_shard)));
        }

        Self {
            connections: conns,
            #[cfg(unix)]
            unix_connections: Arc::new(Pool::new_unbounded()),
            local_limits: RwLock::new(HashMap::new()),
        }
    }

    /// Set a per-upstream local connection limit.
    /// This sets the same local-limit index across all shards for compatibility.
    pub fn set_local_limit(&self, upstream: &UpstreamInner, limit: usize) -> usize {
        let mut limits = self
            .local_limits
            .write()
            .expect("local_limits lock poisoned");
        if let Some(&idx) = limits.get(upstream) {
            return idx;
        }
        let mut idx = 0usize;
        for (i, pool) in self.connections.iter().enumerate() {
            let ret = pool.set_local_limit(limit);
            if i == 0 {
                idx = ret;
            }
        }
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
        let shard = self.shard_for_key(&key);
        self.connections[shard].pull(key).await
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
        let shard = self.shard_for_key(&key);
        self.connections[shard]
            .pull_with_wait_local_limit(key, local_limit_idx)
            .await
    }

    pub fn connections(&self) -> &Arc<Pool<PoolKey, SendRequestWrapper>> {
        // Return first shard for compatibility with existing call sites
        &self.connections[0]
    }

    #[cfg(unix)]
    pub fn unix_connections(&self) -> &Arc<Pool<PoolKey, SendRequestWrapper>> {
        &self.unix_connections
    }

    /// Select the sharded pool for the given upstream/client key.
    pub fn select_pool(
        &self,
        upstream: &UpstreamInner,
        client_ip: Option<IpAddr>,
    ) -> &Arc<Pool<PoolKey, SendRequestWrapper>> {
        #[cfg(unix)]
        if upstream.proxy_unix.is_some() {
            return &self.unix_connections;
        }
        let key = (upstream.clone(), client_ip);
        &self.connections[self.shard_for_key(&key)]
    }

    fn shard_for_key(&self, key: &PoolKey) -> usize {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        (hasher.finish() as usize) % self.connections.len()
    }
}
