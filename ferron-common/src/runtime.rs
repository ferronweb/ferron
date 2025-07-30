use std::future::Future;

/// Spawn a future in an asynchronous runtime
#[cfg(feature = "runtime-monoio")]
pub fn spawn(future: impl Future + 'static) {
  monoio::spawn(future);
}

/// Spawn a future in an asynchronous runtime
#[cfg(feature = "runtime-tokio")]
pub fn spawn(future: impl Future + 'static) {
  tokio::task::spawn_local(future);
}

#[cfg(feature = "runtime-monoio")]
pub use monoio::spawn_blocking;
#[cfg(feature = "runtime-tokio")]
pub use tokio::task::spawn_blocking;

#[cfg(feature = "runtime-monoio")]
pub use monoio::time::sleep;
#[cfg(feature = "runtime-tokio")]
pub use tokio::time::sleep;

#[cfg(feature = "runtime-monoio")]
pub use monoio::time::timeout;
#[cfg(feature = "runtime-tokio")]
pub use tokio::time::timeout;

#[cfg(feature = "runtime-monoio")]
pub use monoio::select;
#[cfg(feature = "runtime-tokio")]
pub use tokio::select;
