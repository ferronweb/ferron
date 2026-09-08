//! Secondary Tokio runtime handle for blocking/distributed backends.
//!
//! Request stages run on primary `zincio` threads. The `redis` crate
//! requires a Tokio runtime, so all Redis I/O is spawned onto the
//! secondary Tokio runtime captured during `Module::start` (same pattern
//! as `http-proxy` and `http-cache`).

use std::sync::OnceLock;

/// Captured secondary runtime handle.
pub static SECONDARY_HANDLE: OnceLock<tokio::runtime::Handle> = OnceLock::new();

/// Try to get the captured secondary handle, if `Module::start` ran.
#[inline]
pub fn try_get_secondary_handle() -> Option<tokio::runtime::Handle> {
    SECONDARY_HANDLE.get().cloned()
}

/// Capture the secondary handle from inside `Runtime::block_on`.
///
/// Must be called from `Module::start` via `runtime.block_on(...)`, where
/// `Handle::current()` refers to the secondary Tokio runtime.
#[inline]
pub fn capture_secondary_handle(runtime: &ferron_core::runtime::Runtime) -> tokio::runtime::Handle {
    let handle = runtime.block_on(async move { tokio::runtime::Handle::current() });
    let _ = SECONDARY_HANDLE.set(handle.clone());
    handle
}
