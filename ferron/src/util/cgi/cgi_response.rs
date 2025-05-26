use memchr::memmem::Finder;
use std::io::Error;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncReadExt, ReadBuf};

/// Constant defining the capacity of the response buffer
const RESPONSE_BUFFER_CAPACITY: usize = 16384;

/// Struct representing a response, which wraps an async read stream
pub struct CgiResponse<R>
where
  R: AsyncRead + Unpin,
{
  stream: R,
  response_buf: Vec<u8>,
  response_head_length: Option<usize>,
}

impl<R> CgiResponse<R>
where
  R: AsyncRead + Unpin,
{
  /// Constructor to create a new CgiResponse instance
  pub fn new(stream: R) -> Self {
    Self {
      stream,
      response_buf: Vec::with_capacity(RESPONSE_BUFFER_CAPACITY),
      response_head_length: None,
    }
  }

  /// Asynchronous method to get the response headers
  pub async fn get_head(&mut self) -> Result<&[u8], Error> {
    let mut temp_buf = [0u8; RESPONSE_BUFFER_CAPACITY];
    let rnrn = Finder::new(b"\r\n\r\n");
    let nrnr = Finder::new(b"\n\r\n\r");
    let nn = Finder::new(b"\n\n");
    let rr = Finder::new(b"\r\r");
    let to_parse_length;

    loop {
      // Read data from the stream into the temporary buffer
      let read_bytes = self.stream.read(&mut temp_buf).await?;

      // If no bytes are read, return an empty response head
      if read_bytes == 0 {
        self.response_head_length = Some(0);
        return Ok(&[0u8; 0]);
      }

      // If the response buffer exceeds the capacity, return an empty response head
      if self.response_buf.len() + read_bytes > RESPONSE_BUFFER_CAPACITY {
        self.response_head_length = Some(0);
        return Ok(&[0u8; 0]);
      }

      // Determine the starting point for searching the "\r\n\r\n" sequence
      let begin_rnrn_or_nrnr_search = self.response_buf.len().saturating_sub(3);
      let begin_rr_or_nn_search = self.response_buf.len().saturating_sub(1);
      self.response_buf.extend_from_slice(&temp_buf[..read_bytes]);

      // Search for the "\r\n\r\n" sequence in the response buffer
      if let Some(rnrn_index) = rnrn.find(&self.response_buf[begin_rnrn_or_nrnr_search..]) {
        to_parse_length = begin_rnrn_or_nrnr_search + rnrn_index + 4;
        break;
      } else if let Some(nrnr_index) = nrnr.find(&self.response_buf[begin_rnrn_or_nrnr_search..]) {
        to_parse_length = begin_rnrn_or_nrnr_search + nrnr_index + 4;
        break;
      } else if let Some(nn_index) = nn.find(&self.response_buf[begin_rr_or_nn_search..]) {
        to_parse_length = begin_rr_or_nn_search + nn_index + 2;
        break;
      } else if let Some(rr_index) = rr.find(&self.response_buf[begin_rr_or_nn_search..]) {
        to_parse_length = begin_rr_or_nn_search + rr_index + 2;
        break;
      }
    }

    // Set the length of the response header
    self.response_head_length = Some(to_parse_length);

    // Return the response header as a byte slice
    Ok(&self.response_buf[..to_parse_length])
  }
}

// Implementation of AsyncRead for the CgiResponse struct
impl<R> AsyncRead for CgiResponse<R>
where
  R: AsyncRead + Unpin,
{
  fn poll_read(
    mut self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    buf: &mut ReadBuf<'_>,
  ) -> Poll<std::io::Result<()>> {
    // If the response header length is known and the buffer contains more data than the header length
    if let Some(response_head_length) = self.response_head_length {
      if self.response_buf.len() > response_head_length {
        let remaining_data = &self.response_buf[response_head_length..];
        let to_read = remaining_data.len().min(buf.remaining());
        buf.put_slice(&remaining_data[..to_read]);
        self.response_head_length = Some(response_head_length + to_read);
        return Poll::Ready(Ok(()));
      }
    }

    // Create a temporary buffer to hold the data to be consumed
    let stream = Pin::new(&mut self.stream);
    match stream.poll_read(cx, buf) {
      Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
      other => other,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use tokio::io::AsyncReadExt;
  use tokio_test::io::Builder;

  #[tokio::test]
  async fn test_get_head() {
    let data = b"Content-Type: text/plain\r\n\r\n";
    let mut stream = Builder::new().read(data).build();
    let mut response = CgiResponse::new(&mut stream);

    let head = response.get_head().await.unwrap();
    assert_eq!(head, b"Content-Type: text/plain\r\n\r\n");
  }

  #[tokio::test]
  async fn test_get_head_nn() {
    let data = b"Content-Type: text/plain\n\n";
    let mut stream = Builder::new().read(data).build();
    let mut response = CgiResponse::new(&mut stream);

    let head = response.get_head().await.unwrap();
    assert_eq!(head, b"Content-Type: text/plain\n\n");
  }

  #[tokio::test]
  async fn test_get_head_large_headers() {
    let data = b"Content-Type: text/plain\r\n";
    let large_header = vec![b'A'; RESPONSE_BUFFER_CAPACITY + 10]
      .into_iter()
      .collect::<Vec<u8>>();
    let mut stream = Builder::new().read(data).read(&large_header).build();
    let mut response = CgiResponse::new(&mut stream);

    let result = response.get_head().await;
    assert_eq!(result.unwrap().len(), 0);

    // Consume the remaining data to avoid panicking
    let mut remaining_data = vec![0u8; RESPONSE_BUFFER_CAPACITY + 10];
    let _ = response.stream.read(&mut remaining_data).await;
  }

  #[tokio::test]
  async fn test_get_head_premature_eof() {
    let data = b"Content-Type: text/plain\r\n";
    let mut stream = Builder::new().read(data).build();
    let mut response = CgiResponse::new(&mut stream);

    let result = response.get_head().await;
    assert_eq!(result.unwrap().len(), 0);
  }

  #[tokio::test]
  async fn test_poll_read() {
    let data = b"Content-Type: text/plain\r\n\r\nHello, world!";
    let mut stream = Builder::new().read(data).build();
    let mut response = CgiResponse::new(&mut stream);

    let head = response.get_head().await.unwrap();
    assert_eq!(head, b"Content-Type: text/plain\r\n\r\n");

    let mut buf = vec![0u8; 13];
    let n = response.read(&mut buf).await.unwrap();
    assert_eq!(n, 13);
    assert_eq!(&buf[..n], b"Hello, world!");
  }
}
