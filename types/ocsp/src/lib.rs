//! OCSP stapling support for Ferron TLS servers.
//!
//! This crate provides:
//! - `OcspStapler`: a `ResolvesServerCert` wrapper that attaches OCSP responses
//! - `OcspServiceHandle`: shared handle to the background OCSP fetching service
//! - `take_ocsp_startup_state()`: consume startup pieces so a module can spawn the background task
//! - `get_service_handle()`: global accessor for TLS providers
//!
//! # Architecture
//!
//! A single background task runs in a module-owned runtime (the `ocsp-stapler`
//! module owns the task and its heavy dependencies). The task fetches OCSP
//! responses over HTTPS and caches them. TLS providers wrap their certificate
//! resolver with `OcspStapler`, which intercepts `resolve()` calls and attaches
//! stapled responses from the cache.
//!
//! # Usage
//!
//! 1. Configure an event sink with `set_event_sink(...)` before startup so logs and metrics can be emitted.
//! 2. The `ocsp-stapler` module should call `take_ocsp_startup_state()` during its ModuleLoader startup
//!    and spawn the returned receiver task on the module's runtime.
//! 3. In your TLS provider, call `get_service_handle()` and wrap your resolver:
//!    ```ignore
//!    if let Some(handle) = ferron_ocsp::get_service_handle() {
//!        config.cert_resolver = Arc::new(OcspStapler::new(inner_resolver, &handle));
//!    }
//!    ```

use std::collections::HashMap;
use std::sync::Arc;

use ferron_observability::CompositeEventSink;
use parking_lot::RwLock;
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

// Type alias for the OCSP cache to reduce type complexity
type OcspCache = Arc<RwLock<HashMap<Vec<u8>, Option<Arc<CertifiedKey>>>>>;

/// Error returned when `init_ocsp_service` is called more than once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlreadyInitialized;

impl std::fmt::Display for AlreadyInitialized {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "OCSP service already initialized")
    }
}

impl std::error::Error for AlreadyInitialized {}

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

/// Eagerly-created global state. The channel is created on first access so
/// certs can be queued via the sender even before `init_ocsp_service` spawns
/// the background task.
struct GlobalState {
    sender: mpsc::UnboundedSender<CertifiedKey>,
    receiver: std::sync::Mutex<Option<mpsc::UnboundedReceiver<CertifiedKey>>>,
    cache: OcspCache,
    cancel_token: CancellationToken,
    event_sink: parking_lot::Mutex<Option<Arc<CompositeEventSink>>>,
}

static GLOBAL_STATE: std::sync::OnceLock<GlobalState> = std::sync::OnceLock::new();

fn get_or_init_global() -> &'static GlobalState {
    GLOBAL_STATE.get_or_init(|| {
        let (sender, receiver) = mpsc::unbounded_channel();
        GlobalState {
            sender,
            receiver: std::sync::Mutex::new(Some(receiver)),
            cache: Arc::new(RwLock::new(HashMap::new())),
            cancel_token: CancellationToken::new(),
            event_sink: parking_lot::Mutex::new(None),
        }
    })
}

/// Set the event sink for the OCSP service. Call before `init_ocsp_service`.
///
/// This allows the OCSP background task to emit log events through the
/// observability system instead of using `log_*` macros directly.
pub fn set_event_sink(event_sink: Arc<CompositeEventSink>) {
    let state = get_or_init_global();
    *state.event_sink.lock() = Some(event_sink);
}

/// Take the startup pieces required to spawn the OCSP background task from
/// another crate (e.g., the ocsp-stapler module). This consumes the receiver
/// so the caller is responsible for spawning the background task. Returns
/// `Err(AlreadyInitialized)` if the receiver was already taken (service
/// already initialized).
#[allow(clippy::type_complexity)]
pub fn take_ocsp_startup_state() -> Result<
    (
        mpsc::UnboundedReceiver<CertifiedKey>,
        OcspCache,
        CancellationToken,
        Option<Arc<CompositeEventSink>>,
    ),
    AlreadyInitialized,
> {
    let state = get_or_init_global();
    let receiver = state
        .receiver
        .lock()
        .unwrap()
        .take()
        .ok_or(AlreadyInitialized)?;
    let cache = state.cache.clone();
    let cancel_token = state.cancel_token.clone();
    let event_sink = state.event_sink.lock().clone();
    Ok((receiver, cache, cancel_token, event_sink))
}

/// Get the global `OcspServiceHandle`.
///
/// Always returns `Some` — the channel and cache are created on first access.
/// Certs can be queued via the returned handle even before `init_ocsp_service`
/// spawns the background task.
pub fn get_service_handle() -> Option<OcspServiceHandle> {
    let state = get_or_init_global();
    Some(OcspServiceHandle {
        sender: state.sender.clone(),
        cache: state.cache.clone(),
        cancel_token: state.cancel_token.clone(),
        event_sink: state.event_sink.lock().clone(),
    })
}

// ---------------------------------------------------------------------------
// Shared handle
// ---------------------------------------------------------------------------

/// Cheap to clone (`Arc`-backed channels and locks).
#[derive(Clone)]
pub struct OcspServiceHandle {
    sender: mpsc::UnboundedSender<CertifiedKey>,
    cache: OcspCache,
    #[allow(dead_code)]
    cancel_token: CancellationToken,
    #[allow(dead_code)]
    event_sink: Option<Arc<CompositeEventSink>>,
}

impl OcspServiceHandle {
    /// Send a `CertifiedKey` to the background task for OCSP fetching.
    pub fn preload(&self, key: CertifiedKey) {
        if !key.cert.is_empty() {
            let _ = self.sender.send(key);
        }
    }
}

// ---------------------------------------------------------------------------
// OcspStapler — ResolvesServerCert wrapper
// ---------------------------------------------------------------------------

/// Wraps an inner `ResolvesServerCert` and attaches OCSP responses from the
/// shared cache.
///
/// On the first `resolve()` call for a given certificate, the original key is
/// returned and a fetch is triggered in the background. Subsequent calls
/// return the key with the stapled OCSP response attached.
#[derive(Debug)]
pub struct OcspStapler {
    inner: Arc<dyn ResolvesServerCert>,
    cache: OcspCache,
    sender: mpsc::UnboundedSender<CertifiedKey>,
}

impl OcspStapler {
    /// Create a new `OcspStapler` wrapping `inner`.
    pub fn new(inner: Arc<dyn ResolvesServerCert>, handle: &OcspServiceHandle) -> Self {
        Self {
            inner,
            cache: handle.cache.clone(),
            sender: handle.sender.clone(),
        }
    }
}

impl ResolvesServerCert for OcspStapler {
    fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        let original_key = self.inner.resolve(client_hello)?;
        if let Some(leaf) = original_key.cert.first() {
            let leaf_bytes: Vec<u8> = leaf.to_vec();

            // Read cache — uses parking_lot::RwLock which is safe to call from
            // any thread (including vibeio primary threads).
            let cached = self.cache.read();

            if let Some(cached_entry) = cached.get(&leaf_bytes) {
                if let Some(stapled) = cached_entry {
                    return Some(stapled.clone());
                }
                // Entry exists but has no OCSP yet — return original without re-triggering
            } else {
                // Not in cache yet — trigger fetch
                drop(cached);
                let _ = self.sender.send((*original_key).clone());
            }
        }
        Some(original_key)
    }
}
