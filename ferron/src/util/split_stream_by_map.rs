// Copyright (c) Andrew Burkett
// Portions of this file are derived from `split-stream-by` (https://github.com/drewkett/split-stream-by).
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.

use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Poll, Waker};

use futures_util::future::Either;
use futures_util::stream::Stream;
use pin_project_lite::pin_project;
use tokio::sync::Mutex;

pin_project! {
struct SplitByMap<I, L, R, S, P> {
    buf_left: Option<L>,
    buf_right: Option<R>,
    waker_left: Option<Waker>,
    waker_right: Option<Waker>,
    #[pin]
    stream: S,
    predicate: P,
    item: PhantomData<I>,
}
}

impl<I, L, R, S, P> SplitByMap<I, L, R, S, P>
where
  S: Stream<Item = I>,
  P: Fn(I) -> Either<L, R>,
{
  fn new(stream: S, predicate: P) -> Arc<Mutex<Self>> {
    Arc::new(Mutex::new(Self {
      buf_right: None,
      buf_left: None,
      waker_right: None,
      waker_left: None,
      stream,
      predicate,
      item: PhantomData,
    }))
  }

  fn poll_next_left(
    self: std::pin::Pin<&mut Self>,
    cx: &mut futures_util::task::Context<'_>,
  ) -> std::task::Poll<Option<L>> {
    let this = self.project();
    // Assign the waker multiple times, because if it was only once, the waking might fail
    *this.waker_left = Some(cx.waker().clone());
    if let Some(item) = this.buf_left.take() {
      // There was already a value in the buffer. Return that value
      return Poll::Ready(Some(item));
    }
    if this.buf_right.is_some() {
      // There is a value available for the other stream. Wake that stream if possible
      // and return pending since we can't store multiple values for a stream
      if let Some(waker) = this.waker_right {
        waker.wake_by_ref();
      }
      return Poll::Pending;
    }
    match this.stream.poll_next(cx) {
      Poll::Ready(Some(item)) => {
        match (this.predicate)(item) {
          Either::Left(left_item) => Poll::Ready(Some(left_item)),
          Either::Right(right_item) => {
            // This value is not what we wanted. Store it and notify other partition
            // task if it exists
            let _ = this.buf_right.replace(right_item);
            if let Some(waker) = this.waker_right {
              waker.wake_by_ref();
            }
            Poll::Pending
          }
        }
      }
      Poll::Ready(None) => {
        // If the underlying stream is finished, the `right` stream also must be
        // finished, so wake it in case nothing else polls it
        if let Some(waker) = this.waker_right {
          waker.wake_by_ref();
        }
        Poll::Ready(None)
      }
      Poll::Pending => Poll::Pending,
    }
  }

  fn poll_next_right(
    self: std::pin::Pin<&mut Self>,
    cx: &mut futures_util::task::Context<'_>,
  ) -> std::task::Poll<Option<R>> {
    let this = self.project();
    // Assign the waker multiple times, because if it was only once, the waking might fail
    *this.waker_right = Some(cx.waker().clone());
    if let Some(item) = this.buf_right.take() {
      // There was already a value in the buffer. Return that value
      return Poll::Ready(Some(item));
    }
    if this.buf_left.is_some() {
      // There is a value available for the other stream. Wake that stream if possible
      // and return pending since we can't store multiple values for a stream
      if let Some(waker) = this.waker_left {
        waker.wake_by_ref();
      }
      return Poll::Pending;
    }
    match this.stream.poll_next(cx) {
      Poll::Ready(Some(item)) => {
        match (this.predicate)(item) {
          Either::Left(left_item) => {
            // This value is not what we wanted. Store it and notify other partition
            // task if it exists
            let _ = this.buf_left.replace(left_item);
            if let Some(waker) = this.waker_left {
              waker.wake_by_ref();
            }
            Poll::Pending
          }
          Either::Right(right_item) => Poll::Ready(Some(right_item)),
        }
      }
      Poll::Ready(None) => {
        // If the underlying stream is finished, the `left` stream also must be
        // finished, so wake it in case nothing else polls it
        if let Some(waker) = this.waker_left {
          waker.wake_by_ref();
        }
        Poll::Ready(None)
      }
      Poll::Pending => Poll::Pending,
    }
  }
}

/// A struct that implements `Stream` which returns the inner values where
/// the predicate returns `Either::Left(..)` when using `split_by_map`
#[allow(clippy::type_complexity)]
pub struct LeftSplitByMap<I, L, R, S, P> {
  stream: Arc<Mutex<SplitByMap<I, L, R, S, P>>>,
}

impl<I, L, R, S, P> LeftSplitByMap<I, L, R, S, P> {
  #[allow(clippy::type_complexity)]
  fn new(stream: Arc<Mutex<SplitByMap<I, L, R, S, P>>>) -> Self {
    Self { stream }
  }
}

impl<I, L, R, S, P> Stream for LeftSplitByMap<I, L, R, S, P>
where
  S: Stream<Item = I> + Unpin,
  P: Fn(I) -> Either<L, R>,
{
  type Item = L;
  fn poll_next(
    self: std::pin::Pin<&mut Self>,
    cx: &mut futures_util::task::Context<'_>,
  ) -> std::task::Poll<Option<Self::Item>> {
    let response = if let Ok(mut guard) = self.stream.try_lock() {
      SplitByMap::poll_next_left(Pin::new(&mut guard), cx)
    } else {
      cx.waker().wake_by_ref();
      Poll::Pending
    };
    response
  }
}

/// A struct that implements `Stream` which returns the inner values where
/// the predicate returns `Either::Right(..)` when using `split_by_map`
#[allow(clippy::type_complexity)]
pub struct RightSplitByMap<I, L, R, S, P> {
  stream: Arc<Mutex<SplitByMap<I, L, R, S, P>>>,
}

impl<I, L, R, S, P> RightSplitByMap<I, L, R, S, P> {
  #[allow(clippy::type_complexity)]
  fn new(stream: Arc<Mutex<SplitByMap<I, L, R, S, P>>>) -> Self {
    Self { stream }
  }
}

impl<I, L, R, S, P> Stream for RightSplitByMap<I, L, R, S, P>
where
  S: Stream<Item = I> + Unpin,
  P: Fn(I) -> Either<L, R>,
{
  type Item = R;
  fn poll_next(
    self: std::pin::Pin<&mut Self>,
    cx: &mut futures_util::task::Context<'_>,
  ) -> std::task::Poll<Option<Self::Item>> {
    let response = if let Ok(mut guard) = self.stream.try_lock() {
      SplitByMap::poll_next_right(Pin::new(&mut guard), cx)
    } else {
      cx.waker().wake_by_ref();
      Poll::Pending
    };
    response
  }
}

/// This extension trait provides the functionality for splitting a
/// stream by a predicate of type `Fn(Self::Item) -> Either<L,R>`. The resulting
/// streams will yield types `L` and `R` respectively
pub trait SplitStreamByMapExt<P, L, R>: Stream {
  /// This takes ownership of a stream and returns two streams based on a
  /// predicate. The predicate takes an item by value and returns
  /// `Either::Left(..)` or `Either::Right(..)` where the inner
  /// values of `Left` and `Right` become the items of the two respective
  /// streams
  ///
  /// ```
  /// use split_stream_by::{Either, SplitStreamByMapExt};
  /// struct Request {
  ///   //...
  /// }
  /// struct Response {
  ///   //...
  /// }
  /// enum Message {
  ///   Request(Request),
  ///   Response(Response)
  /// }
  /// let incoming_stream = futures::stream::iter([
  ///   Message::Request(Request {}),
  ///   Message::Response(Response {}),
  ///   Message::Response(Response {}),
  /// ]);
  /// let (mut request_stream, mut response_stream) = incoming_stream.split_by_map(|item| match item {
  ///   Message::Request(req) => Either::Left(req),
  ///   Message::Response(res) => Either::Right(res),
  /// });
  /// ```
  #[allow(clippy::type_complexity)]
  fn split_by_map(
    self,
    predicate: P,
  ) -> (
    LeftSplitByMap<Self::Item, L, R, Self, P>,
    RightSplitByMap<Self::Item, L, R, Self, P>,
  )
  where
    P: Fn(Self::Item) -> Either<L, R>,
    Self: Sized,
  {
    let stream = SplitByMap::new(self, predicate);
    let true_stream = LeftSplitByMap::new(stream.clone());
    let false_stream = RightSplitByMap::new(stream);
    (true_stream, false_stream)
  }
}

impl<T, P, L, R> SplitStreamByMapExt<P, L, R> for T where T: Stream + ?Sized {}

#[cfg(test)]
mod tests {
  use super::*;
  use futures_util::{stream, StreamExt};

  #[tokio::test]
  async fn test_split_by_map_basic() {
    let input_stream = stream::iter(vec![1, 2, 3, 4, 5, 6]);
    let (evens, odds) = input_stream.split_by_map(|x| {
      if x % 2 == 0 {
        Either::Left(x)
      } else {
        Either::Right(x)
      }
    });

    tokio::spawn(async move {
      let evens_collected: Vec<i32> = evens.collect().await;
      assert_eq!(evens_collected, vec![2, 4, 6]);
    });

    tokio::spawn(async move {
      let odds_collected: Vec<i32> = odds.collect().await;
      assert_eq!(odds_collected, vec![1, 3, 5]);
    });
  }

  #[tokio::test]
  async fn test_split_by_map_empty_stream() {
    let input_stream = stream::iter(Vec::<i32>::new());
    let (left, right) = input_stream.split_by_map(|x| {
      if x % 2 == 0 {
        Either::Left(x)
      } else {
        Either::Right(x)
      }
    });

    tokio::spawn(async move {
      let left_collected: Vec<i32> = left.collect().await;
      assert!(left_collected.is_empty());
    });

    tokio::spawn(async move {
      let right_collected: Vec<i32> = right.collect().await;
      assert!(right_collected.is_empty());
    });
  }

  #[tokio::test]
  async fn test_split_by_map_all_left() {
    let input_stream = stream::iter(vec![2, 4, 6, 8]);
    let (left, right) = input_stream.split_by_map(Either::<i32, i32>::Left);

    tokio::spawn(async move {
      let left_collected: Vec<i32> = left.collect().await;
      assert_eq!(left_collected, vec![2, 4, 6, 8]);
    });

    tokio::spawn(async move {
      let right_collected: Vec<i32> = right.collect().await;
      assert!(right_collected.is_empty());
    });
  }

  #[tokio::test]
  async fn test_split_by_map_all_right() {
    let input_stream = stream::iter(vec![1, 3, 5, 7]);
    let (left, right) = input_stream.split_by_map(Either::<i32, i32>::Right);

    tokio::spawn(async move {
      let left_collected: Vec<i32> = left.collect().await;
      assert!(left_collected.is_empty());
    });

    tokio::spawn(async move {
      let right_collected: Vec<i32> = right.collect().await;
      assert_eq!(right_collected, vec![1, 3, 5, 7]);
    });
  }
}
