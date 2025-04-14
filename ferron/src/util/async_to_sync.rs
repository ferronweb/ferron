use std::future::Future;
use std::task::{Context, Poll, Waker};

pub fn async_to_sync<T>(future: impl Future<Output = T>) -> T {
  let mut future_boxed = Box::pin(future);
  let mut context = Context::from_waker(Waker::noop());
  loop {
    if let Poll::Ready(return_value) = future_boxed.as_mut().poll(&mut context) {
      return return_value;
    }
  }
}
