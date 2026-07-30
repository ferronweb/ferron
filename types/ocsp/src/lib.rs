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

use ferron_observability::{
    CompositeEventSink, Event, MetricAttributeValue, MetricEvent, MetricType, MetricValue,
};
use parking_lot::RwLock;
use rustls::pki_types::CertificateDer;
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

// Type alias for the OCSP cache to reduce type complexity
type OcspCache = Arc<RwLock<HashMap<Vec<u8>, Option<Vec<u8>>>>>;

/// Maps certificate leaf bytes to hostname for per-host OCSP metrics.
pub type OcspHostMap = Arc<RwLock<HashMap<Vec<u8>, String>>>;

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
    sender: mpsc::UnboundedSender<Vec<CertificateDer<'static>>>,
    receiver: std::sync::Mutex<Option<mpsc::UnboundedReceiver<Vec<CertificateDer<'static>>>>>,
    cache: OcspCache,
    host_map: OcspHostMap,
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
            host_map: Arc::new(RwLock::new(HashMap::new())),
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
        mpsc::UnboundedReceiver<Vec<CertificateDer<'static>>>,
        OcspCache,
        OcspHostMap,
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
    let host_map = state.host_map.clone();
    let cancel_token = state.cancel_token.clone();
    let event_sink = state.event_sink.lock().clone();
    Ok((receiver, cache, host_map, cancel_token, event_sink))
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
        host_map: state.host_map.clone(),
    })
}

// ---------------------------------------------------------------------------
// Shared handle
// ---------------------------------------------------------------------------

/// Cheap to clone (`Arc`-backed channels and locks).
#[derive(Clone)]
pub struct OcspServiceHandle {
    sender: mpsc::UnboundedSender<Vec<CertificateDer<'static>>>,
    cache: OcspCache,
    host_map: OcspHostMap,
}

impl OcspServiceHandle {
    /// Send a `Vec<CertificateDer<'static>>` to the background task for OCSP fetching.
    pub fn preload(&self, cert: Vec<CertificateDer<'static>>) {
        if !cert.is_empty() {
            let _ = self.sender.send(cert);
        }
    }

    /// Send a certificate chain to the background task with an associated hostname
    /// for per-host OCSP metrics.
    pub fn preload_with_host(&self, cert: Vec<CertificateDer<'static>>, hostname: String) {
        if let Some(leaf) = cert.first() {
            let leaf_bytes = leaf.to_vec();
            self.host_map.write().insert(leaf_bytes, hostname);
        }
        self.preload(cert);
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
pub struct OcspStapler {
    inner: Arc<dyn ResolvesServerCert>,
    cache: OcspCache,
    sender: mpsc::UnboundedSender<Vec<CertificateDer<'static>>>,
    host_map: OcspHostMap,
    event_sink: Option<Arc<CompositeEventSink>>,
}

impl std::fmt::Debug for OcspStapler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OcspStapler")
            .field("inner", &"<dyn ResolvesServerCert>")
            .finish()
    }
}

impl OcspStapler {
    /// Create a new `OcspStapler` wrapping `inner`.
    pub fn new(inner: Arc<dyn ResolvesServerCert>, handle: &OcspServiceHandle) -> Self {
        Self {
            inner,
            cache: handle.cache.clone(),
            sender: handle.sender.clone(),
            host_map: handle.host_map.clone(),
            event_sink: None,
        }
    }

    /// Set the event sink for per-host metrics emission.
    pub fn with_event_sink(mut self, event_sink: Arc<CompositeEventSink>) -> Self {
        self.event_sink = Some(event_sink);
        self
    }
}

impl ResolvesServerCert for OcspStapler {
    fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        let mut original_key = self.inner.resolve(client_hello)?;
        if let Some(leaf) = original_key.cert.first() {
            let leaf_bytes: Vec<u8> = leaf.to_vec();

            // Read cache — uses parking_lot::RwLock which is safe to call from
            // any thread (including vibeio primary threads).
            let cached = self.cache.read();

            if let Some(cached_entry) = cached.get(&leaf_bytes) {
                if let Some(ocsp) = cached_entry {
                    // Put the OCSP response into the CertifiedKey
                    if let Some(original_key_mut) = Arc::get_mut(&mut original_key) {
                        original_key_mut.ocsp = Some(ocsp.clone());
                    } else {
                        let mut original_key_mut = (*original_key).clone();
                        original_key_mut.ocsp = Some(ocsp.clone());
                        original_key = Arc::new(original_key_mut);
                    }

                    // Emit per-host stapling hit metric
                    if let Some(ref event_sink) = self.event_sink {
                        let host = self
                            .host_map
                            .read()
                            .get(&leaf_bytes)
                            .cloned()
                            .unwrap_or_else(|| "_global".to_string());
                        event_sink.emit(Event::Metric(MetricEvent {
                            name: "ferron.ocsp.stapling.hit_total",
                            attributes: vec![("ferron.host", MetricAttributeValue::String(host))],
                            ty: MetricType::Counter,
                            value: MetricValue::U64(1),
                            unit: Some("{hit}"),
                            description: Some("OCSP responses served to clients"),
                            trace_context: None,
                            control_plane_metadata: None,
                        }));
                    }
                }
                // Entry exists but has no OCSP yet — return original without re-triggering
            } else {
                // Not in cache yet — trigger fetch
                drop(cached);
                let _ = self.sender.send(original_key.cert.clone());
            }
        }
        Some(original_key)
    }
}
