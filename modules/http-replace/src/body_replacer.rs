//! Streaming body replacement utility.
//!
//! Provides `BodyReplacer` which wraps an HTTP body stream and performs
//! byte-level string replacement without buffering the entire body.

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use http_body::Body;
use http_body::Frame;
use memchr::memmem::Finder;
use pin_project_lite::pin_project;

/// A struct that can wrap a `Body` to replace contents.
pub struct BodyReplacer {
    searched: Arc<Vec<u8>>,
    replacement: Arc<Vec<u8>>,
    once: bool,
}

impl BodyReplacer {
    /// Creates a new body replacer.
    pub fn new(searched: &[u8], replacement: &[u8], once: bool) -> Self {
        Self {
            searched: Arc::new(searched.to_vec()),
            replacement: Arc::new(replacement.to_vec()),
            once,
        }
    }

    /// Wraps a `Body` to replace contents.
    pub fn wrap<B>(&self, body: B) -> ReplaceBody<B>
    where
        B: Body,
    {
        ReplaceBody {
            searched: self.searched.clone(),
            replacement: self.replacement.clone(),
            once: self.once,
            replaced: false,
            buffer: None,
            inner: body,
        }
    }
}

pin_project! {
    /// A `Body` with replaced content.
    ///
    /// This struct wraps an inner body stream and performs byte-level
    /// string replacement on data frames as they are polled.
    pub struct ReplaceBody<B> {
        searched: Arc<Vec<u8>>,
        replacement: Arc<Vec<u8>>,
        once: bool,
        replaced: bool,
        buffer: Option<Vec<u8>>,
        #[pin]
        inner: B,
    }
}

impl<B> Body for ReplaceBody<B>
where
    B: Body,
{
    type Data = Bytes;
    type Error = B::Error;

    #[inline]
    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.project();
        let frame_raw = match this.inner.poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => frame,
            Poll::Ready(Some(Err(err))) => return Poll::Ready(Some(Err(err))),
            Poll::Ready(None) => {
                // Flush any remaining buffer
                if let Some(buffer) = this.buffer.take() {
                    return Poll::Ready(Some(Ok(Frame::data(Bytes::from(buffer)))));
                } else {
                    return Poll::Ready(None);
                }
            }
            Poll::Pending => return Poll::Pending,
        };

        let data_result = frame_raw.into_data();
        if let Ok(mut data) = data_result {
            let data_len = data.remaining();
            let data_bytes = data.copy_to_bytes(data_len);

            // If we've already replaced once and once=true, pass through
            if (*this.replaced && *this.once) || this.searched.is_empty() {
                return Poll::Ready(Some(Ok(Frame::data(data_bytes))));
            }

            let mut replaced = Vec::with_capacity(data_len);
            let finder = Finder::new(this.searched.as_slice());

            // Combine buffer with current data
            let combined_bytes = if let Some(buffer) = this.buffer.take() {
                let mut combined_bytes = BytesMut::with_capacity(buffer.len() + data_len);
                combined_bytes.put_slice(&buffer);
                combined_bytes.put_slice(&data_bytes);
                combined_bytes.freeze()
            } else {
                data_bytes
            };

            // Find and replace all occurrences
            let mut last_beg_index = 0;
            while let Some(index) = finder.find(&combined_bytes[last_beg_index..]) {
                replaced.extend_from_slice(&combined_bytes[last_beg_index..last_beg_index + index]);
                replaced.extend_from_slice(this.replacement.as_slice());

                last_beg_index += index + this.searched.len();
                *this.replaced = true;
                if *this.once {
                    break;
                }
            }

            // Handle remaining bytes that might contain a partial match
            if combined_bytes.len() - last_beg_index < this.searched.len() {
                // Keep in buffer - might be partial match
                *this.buffer = Some(combined_bytes[last_beg_index..].to_vec());
            } else {
                // Add all but the last (searched.len() - 1) bytes to output
                replaced.extend_from_slice(
                    &combined_bytes[last_beg_index..combined_bytes.len() - this.searched.len()],
                );
                // Keep the tail in buffer for next frame
                *this.buffer =
                    Some(combined_bytes[combined_bytes.len() - this.searched.len()..].to_vec());
            }

            Poll::Ready(Some(Ok(Frame::data(Bytes::from(replaced)))))
        } else if let Err(frame_raw) = data_result {
            // Handle trailer frames
            let trailers_result = frame_raw.into_trailers();
            if let Ok(trailers) = trailers_result {
                Poll::Ready(Some(Ok(Frame::trailers(trailers))))
            } else {
                unreachable!()
            }
        } else {
            unreachable!()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper function to collect all data from a body
    async fn collect_body_bytes<B>(body: B) -> Result<Vec<u8>, B::Error>
    where
        B: Body + Unpin,
        B::Data: AsRef<[u8]>,
    {
        let mut result = Vec::new();
        let mut body = std::pin::pin!(body);
        while let Some(frame) =
            futures_util::future::poll_fn(|cx| body.as_mut().poll_frame(cx)).await
        {
            let frame = frame?;
            if let Ok(data) = frame.into_data() {
                result.extend_from_slice(data.as_ref());
            }
        }
        Ok(result)
    }

    // Simple test body that yields data in chunks
    struct TestBody {
        chunks: Vec<Bytes>,
        current: usize,
    }

    impl TestBody {
        fn new(chunks: Vec<Vec<u8>>) -> Self {
            Self {
                chunks: chunks.into_iter().map(Bytes::from).collect(),
                current: 0,
            }
        }
    }

    impl Body for TestBody {
        type Data = Bytes;
        type Error = std::convert::Infallible;

        fn poll_frame(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
            if self.current < self.chunks.len() {
                let chunk = self.chunks[self.current].clone();
                self.current += 1;
                Poll::Ready(Some(Ok(Frame::data(chunk))))
            } else {
                Poll::Ready(None)
            }
        }
    }

    #[tokio::test]
    async fn test_basic_replacement() {
        let replacer = BodyReplacer::new(b"world", b"Rust", false);
        let body = TestBody::new(vec![b"Hello world".to_vec()]);
        let replaced_body = replacer.wrap(body);

        let result = collect_body_bytes(replaced_body).await.unwrap();
        assert_eq!(result, b"Hello Rust");
    }

    #[tokio::test]
    async fn test_multiple_replacements() {
        let replacer = BodyReplacer::new(b"foo", b"bar", false);
        let body = TestBody::new(vec![b"foo test foo end foo".to_vec()]);
        let replaced_body = replacer.wrap(body);

        let result = collect_body_bytes(replaced_body).await.unwrap();
        assert_eq!(result, b"bar test bar end bar");
    }

    #[tokio::test]
    async fn test_single_replacement_with_once() {
        let replacer = BodyReplacer::new(b"foo", b"bar", true);
        let body = TestBody::new(vec![b"foo test foo end foo".to_vec()]);
        let replaced_body = replacer.wrap(body);

        let result = collect_body_bytes(replaced_body).await.unwrap();
        assert_eq!(result, b"bar test foo end foo");
    }

    #[tokio::test]
    async fn test_no_matches() {
        let replacer = BodyReplacer::new(b"xyz", b"abc", false);
        let body = TestBody::new(vec![b"Hello world".to_vec()]);
        let replaced_body = replacer.wrap(body);

        let result = collect_body_bytes(replaced_body).await.unwrap();
        assert_eq!(result, b"Hello world");
    }

    #[tokio::test]
    async fn test_empty_body() {
        let replacer = BodyReplacer::new(b"test", b"replaced", false);
        let body = TestBody::new(vec![]);
        let replaced_body = replacer.wrap(body);

        let result = collect_body_bytes(replaced_body).await.unwrap();
        assert_eq!(result, b"");
    }

    #[tokio::test]
    async fn test_pattern_spanning_chunks() {
        let replacer = BodyReplacer::new(b"world", b"Rust", false);
        let body = TestBody::new(vec![b"Hello wo".to_vec(), b"rld test".to_vec()]);
        let replaced_body = replacer.wrap(body);

        let result = collect_body_bytes(replaced_body).await.unwrap();
        assert_eq!(result, b"Hello Rust test");
    }

    #[tokio::test]
    async fn test_multiple_chunks_with_multiple_replacements() {
        let replacer = BodyReplacer::new(b"test", b"demo", false);
        let body = TestBody::new(vec![
            b"test one ".to_vec(),
            b"test two ".to_vec(),
            b"test three".to_vec(),
        ]);
        let replaced_body = replacer.wrap(body);

        let result = collect_body_bytes(replaced_body).await.unwrap();
        assert_eq!(result, b"demo one demo two demo three");
    }

    #[tokio::test]
    async fn test_replacement_longer_than_original() {
        let replacer = BodyReplacer::new(b"a", b"hello", false);
        let body = TestBody::new(vec![b"a b a".to_vec()]);
        let replaced_body = replacer.wrap(body);

        let result = collect_body_bytes(replaced_body).await.unwrap();
        assert_eq!(result, b"hello b hello");
    }

    #[tokio::test]
    async fn test_replacement_shorter_than_original() {
        let replacer = BodyReplacer::new(b"hello", b"hi", false);
        let body = TestBody::new(vec![b"hello world hello".to_vec()]);
        let replaced_body = replacer.wrap(body);

        let result = collect_body_bytes(replaced_body).await.unwrap();
        assert_eq!(result, b"hi world hi");
    }

    #[tokio::test]
    async fn test_empty_replacement() {
        let replacer = BodyReplacer::new(b"remove", b"", false);
        let body = TestBody::new(vec![b"keep remove this remove text".to_vec()]);
        let replaced_body = replacer.wrap(body);

        let result = collect_body_bytes(replaced_body).await.unwrap();
        assert_eq!(result, b"keep  this  text");
    }

    #[tokio::test]
    async fn test_empty_search_pattern() {
        let replacer = BodyReplacer::new(b"", b"X", false);
        let body = TestBody::new(vec![b"test".to_vec()]);
        let replaced_body = replacer.wrap(body);

        let result = collect_body_bytes(replaced_body).await.unwrap();
        // Empty search pattern should not match anything
        assert_eq!(result, b"test");
    }

    #[tokio::test]
    async fn test_single_byte_replacement() {
        let replacer = BodyReplacer::new(b"a", b"X", false);
        let body = TestBody::new(vec![b"banana".to_vec()]);
        let replaced_body = replacer.wrap(body);

        let result = collect_body_bytes(replaced_body).await.unwrap();
        assert_eq!(result, b"bXnXnX");
    }
}
