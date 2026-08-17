use std::sync::{Arc, OnceLock};

use ferron_core::runtime::Runtime;

/// Global accessor for the secondary Tokio runtime handle.
///
/// Populated during `ReverseProxyModule::start()` by spawning a task
/// that captures `tokio::runtime::Handle::current()`.
/// Used for SRV record resolution via `hickory_resolver`.
pub(crate) static SECONDARY_RUNTIME_HANDLE: OnceLock<(
    tokio::runtime::Handle,
    parking_lot::RwLock<Arc<ferron_observability::CompositeEventSink>>,
)> = OnceLock::new();

/// Cache of Hickory DNS resolvers keyed by DNS server IP list.
///
/// Resolvers are reused across SRV lookups that share the same DNS server
/// configuration, avoiding repeated allocation of DNS client state and
/// connection pools. The key is a sorted `Vec<IpAddr>` so that different
/// orderings of the same servers share one resolver.
pub(crate) static RESOLVER_CACHE: OnceLock<
    parking_lot::RwLock<
        rustc_hash::FxHashMap<Vec<std::net::IpAddr>, Arc<hickory_resolver::TokioResolver>>,
    >,
> = OnceLock::new();

/// Returns the secondary runtime handle if it has been captured.
///
/// Returns `None` if `Module::start()` has not been called yet.
#[inline]
pub fn try_get_secondary_runtime_handle() -> Option<(
    tokio::runtime::Handle,
    Arc<ferron_observability::CompositeEventSink>,
)> {
    SECONDARY_RUNTIME_HANDLE
        .get()
        .map(|(h, s)| (h.clone(), s.read().clone()))
}

/// Returns the secondary runtime handle, initializing it if necessary.
///
/// The handle is captured during `Module::start()` by spawning a task
/// on the secondary runtime that calls `tokio::runtime::Handle::current()`.
#[inline]
pub fn get_secondary_runtime_handle(
    runtime: &Runtime,
    sink: Arc<ferron_observability::CompositeEventSink>,
) -> (
    tokio::runtime::Handle,
    Arc<ferron_observability::CompositeEventSink>,
) {
    let sink2 = sink.clone();
    let (h, s) = SECONDARY_RUNTIME_HANDLE.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel();
        runtime.spawn_secondary_task(async move {
            let _ = tx.send(tokio::runtime::Handle::current());
        });
        (
            rx.recv()
                .expect("failed to capture secondary runtime handle"),
            parking_lot::RwLock::new(sink2),
        )
    });
    *s.write() = sink.clone();
    (h.clone(), sink)
}

/// Returns a cached Hickory resolver for the given DNS servers, creating one
/// if it doesn't exist yet.
///
/// Returns `None` if the secondary runtime handle hasn't been captured yet
/// (i.e., `Module::start()` hasn't been called).
#[inline]
pub(crate) fn get_or_create_resolver(
    dns_servers: &[std::net::IpAddr],
) -> Option<Arc<hickory_resolver::TokioResolver>> {
    use hickory_resolver::config::{NameServerConfig, ResolverConfig};
    use hickory_resolver::TokioResolver;

    let mut key = dns_servers.to_vec();
    key.sort();

    // Fast path: check cache with read lock
    if let Some(cache) = RESOLVER_CACHE.get() {
        if let Some(resolver) = cache.read().get(&key) {
            return Some(Arc::clone(resolver));
        }
    }

    // Slow path: build resolver and insert into cache
    let resolver_result = if !dns_servers.is_empty() {
        let mut resolver_config = ResolverConfig::default();
        for server in dns_servers {
            resolver_config.add_name_server(NameServerConfig::udp(*server));
        }
        TokioResolver::builder_with_config(
            resolver_config,
            hickory_resolver::net::runtime::TokioRuntimeProvider::new(),
        )
        .build()
    } else {
        TokioResolver::builder_tokio()
            .unwrap_or_else(|_| {
                TokioResolver::builder_with_config(
                    ResolverConfig::default(),
                    hickory_resolver::net::runtime::TokioRuntimeProvider::new(),
                )
            })
            .build()
    };

    let resolver = match resolver_result {
        Ok(r) => r,
        Err(_) => return None,
    };
    let resolver = Arc::new(resolver);

    let cache = RESOLVER_CACHE.get_or_init(Default::default);
    cache
        .write()
        .entry(key)
        .or_insert_with(|| Arc::clone(&resolver));

    Some(resolver)
}
