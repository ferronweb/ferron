use pin_project_lite::pin_project;
use std::{
  future::Future,
  pin::Pin,
  task::{Context, Poll},
};
use tokio::runtime::Handle;

pin_project! {
    /// A future that executes within a specific Tokio runtime.
    ///
    /// This struct ensures that the wrapped future (`fut`) is polled within the context of the provided Tokio runtime handle (`runtime`).
    pub struct WithRuntime<F> {
        runtime: Handle,
        #[pin]
        fut: F,
    }
}

impl<F> WithRuntime<F> {
  /// Creates a new `WithRuntime` instance.
  ///
  /// # Parameters
  ///
  /// - `runtime`: A `Handle` to the Tokio runtime in which the future should be executed.
  /// - `fut`: The future to be executed within the specified runtime.
  ///
  /// # Returns
  ///
  /// A `WithRuntime` object encapsulating the provided runtime handle and future.
  pub fn new(runtime: Handle, fut: F) -> Self {
    Self { runtime, fut }
  }
}

impl<F> Future for WithRuntime<F>
where
  F: Future,
{
  type Output = F::Output;

  /// Polls the wrapped future within the context of the specified Tokio runtime.
  ///
  /// # Parameters
  ///
  /// - `ctx`: The current task context.
  ///
  /// # Returns
  ///
  /// A `Poll` indicating the state of the wrapped future (`Pending` or `Ready`).
  fn poll(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Self::Output> {
    let this = self.project();
    let _guard = this.runtime.enter();
    this.fut.poll(ctx)
  }
}
