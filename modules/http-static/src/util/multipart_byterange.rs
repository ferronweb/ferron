use std::collections::VecDeque;
use std::task::Poll;

use bytes::Bytes;
use futures_core::Stream;
use http_body::Body;
use pin_project_lite::pin_project;

use crate::util::file_stream::FileStream;

pin_project! {
    pub struct MultipartByterangeBody {
        boundary: String,
        file_length: u64,
        content_type: Option<String>,
        ranges_left: VecDeque<(u64, u64)>,
        file: FileStream,
        #[pin]
        current_stream: Option<FileStream>,
    }
}

impl MultipartByterangeBody {
    #[inline]
    pub fn new(
        boundary: String,
        file_length: u64,
        content_type: Option<String>,
        ranges: Vec<(u64, u64)>,
        file: FileStream,
    ) -> Self {
        Self {
            boundary,
            file_length,
            content_type,
            ranges_left: ranges.into(),
            file,
            current_stream: None,
        }
    }
}

impl Body for MultipartByterangeBody {
    type Data = Bytes;
    type Error = std::io::Error;

    #[inline]
    fn poll_frame(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        let had_current_stream = self.current_stream.is_some();

        // Check for data from the current file stream
        {
            let this = self.as_mut().project();

            if let Some(current_stream) = this.current_stream.as_pin_mut() {
                match current_stream.poll_next(cx) {
                    Poll::Ready(Some(result)) => {
                        return Poll::Ready(Some(result.map(http_body::Frame::data)))
                    }
                    Poll::Pending => return Poll::Pending,
                    _ => {}
                }
            }
        }

        if had_current_stream {
            // Current stream is finished, remove it.
            self.current_stream.take();
        }

        if let Some((start, end)) = self.ranges_left.pop_front() {
            // There are still ranges left, populate the stream and send multipart head
            let end = end.min(self.file_length - 1);
            self.current_stream = Some(self.file.clone_stream(start, Some(end + 1)));
            let mut multipart_head = String::new();
            if had_current_stream {
                multipart_head.push_str("\r\n");
            }
            multipart_head.push_str(&format!(
                "--{}\r\ncontent-range: bytes {start}-{end}/{}\r\n",
                self.boundary, self.file_length
            ));
            if let Some(content_type) = &self.content_type {
                multipart_head.push_str(&format!("content-type: {content_type}\r\n"));
            }
            multipart_head.push_str("\r\n");

            Poll::Ready(Some(Ok(http_body::Frame::data(Bytes::from(
                multipart_head,
            )))))
        } else if had_current_stream {
            // The last part has been read, send the final boundary and close the stream.
            let final_boundary = format!("\r\n--{}--\r\n", self.boundary);
            Poll::Ready(Some(Ok(http_body::Frame::data(Bytes::from(
                final_boundary,
            )))))
        } else {
            // No more multipart data to send
            Poll::Ready(None)
        }
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        self.ranges_left.is_empty() && self.current_stream.is_none()
    }
}
