use std::io::Read;
use std::sync::Arc;

use bytes::BufMut;
use interprocess::unnamed_pipe::{Recver, Sender};
use tokio::sync::Mutex;

use super::{ProcessPoolToServerMessage, ServerToProcessPoolMessage};
use crate::util::{read_ipc_message, write_ipc_message};

pub struct WsgidBodyReader {
  ipc_tx: Arc<Mutex<Sender>>,
  ipc_rx: Arc<Mutex<Recver>>,
  buffer: Vec<u8>,
  finished: bool,
}

impl WsgidBodyReader {
  pub fn new(ipc_tx: Arc<Mutex<Sender>>, ipc_rx: Arc<Mutex<Recver>>) -> Self {
    Self {
      ipc_tx,
      ipc_rx,
      buffer: Vec::new(),
      finished: false,
    }
  }
}

impl Read for WsgidBodyReader {
  fn read(&mut self, mut buf: &mut [u8]) -> std::io::Result<usize> {
    if !self.buffer.is_empty() {
      let slice_len = std::cmp::min(buf.remaining_mut(), self.buffer.len());
      buf.put_slice(&self.buffer[0..slice_len]);
      self.buffer.clear();
      Ok(slice_len)
    } else if self.finished {
      Ok(0)
    } else {
      let rx = &mut self.ipc_rx.blocking_lock();
      let tx = &mut self.ipc_tx.blocking_lock();

      let mut body_fill_with = Vec::new();

      loop {
        write_ipc_message(
          tx,
          &postcard::to_allocvec::<ProcessPoolToServerMessage>(&ProcessPoolToServerMessage {
            application_id: None,
            status_code: None,
            headers: None,
            body_chunk: None,
            error_log_line: None,
            error_message: None,
            requests_body_chunk: true,
          })
          .map_err(|e| std::io::Error::other(e.to_string()))?,
        )?;

        let received_message = postcard::from_bytes::<ServerToProcessPoolMessage>(&read_ipc_message(rx)?)
          .map_err(|e| std::io::Error::other(e.to_string()))?;
        if let Some(body_error_message) = received_message.body_error_message {
          return Err(std::io::Error::other(body_error_message));
        } else if let Some(body_chunk) = received_message.body_chunk {
          body_fill_with.extend_from_slice(&body_chunk);
          if body_fill_with.len() >= buf.remaining_mut() {
            break;
          }
        } else {
          self.finished = true;
          break;
        }
      }

      let slice_len = std::cmp::min(buf.len(), body_fill_with.len());
      buf.put_slice(&body_fill_with[0..slice_len]);
      if slice_len < body_fill_with.len() {
        self
          .buffer
          .extend_from_slice(&body_fill_with[slice_len..body_fill_with.len()]);
      }
      Ok(slice_len)
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_read_from_ipc() {
    let (tx_inner, mut rx_outer) = interprocess::unnamed_pipe::pipe().unwrap();
    let (mut tx_outer, rx_inner) = interprocess::unnamed_pipe::pipe().unwrap();
    let mut reader = WsgidBodyReader::new(Arc::new(Mutex::new(tx_inner)), Arc::new(Mutex::new(rx_inner)));
    let input = b"Some data";
    write_ipc_message(
      &mut tx_outer,
      &postcard::to_allocvec(&ServerToProcessPoolMessage {
        application_id: None,
        environment_variables: None,
        body_chunk: Some(input.to_vec()),
        body_error_message: None,
        requests_body_chunk: false,
      })
      .unwrap(),
    )
    .unwrap();
    write_ipc_message(
      &mut tx_outer,
      &postcard::to_allocvec(&ServerToProcessPoolMessage {
        application_id: None,
        environment_variables: None,
        body_chunk: None,
        body_error_message: None,
        requests_body_chunk: false,
      })
      .unwrap(),
    )
    .unwrap();

    let mut buffer = [0u8; 128];
    let read_bytes = reader.read(&mut buffer).unwrap();
    assert_eq!(read_bytes, input.len());
    assert_eq!(&input[0..read_bytes], input);

    let received_message =
      postcard::from_bytes::<ProcessPoolToServerMessage>(&read_ipc_message(&mut rx_outer).unwrap()).unwrap();
    assert!(received_message.requests_body_chunk);
    let received_message =
      postcard::from_bytes::<ProcessPoolToServerMessage>(&read_ipc_message(&mut rx_outer).unwrap()).unwrap();
    assert!(received_message.requests_body_chunk);
  }

  #[test]
  fn test_empty_input() {
    let (tx_inner, _rx_outer) = interprocess::unnamed_pipe::pipe().unwrap();
    let (mut tx_outer, rx_inner) = interprocess::unnamed_pipe::pipe().unwrap();
    let mut reader = WsgidBodyReader::new(Arc::new(Mutex::new(tx_inner)), Arc::new(Mutex::new(rx_inner)));
    write_ipc_message(
      &mut tx_outer,
      &postcard::to_allocvec(&ServerToProcessPoolMessage {
        application_id: None,
        environment_variables: None,
        body_chunk: None,
        body_error_message: None,
        requests_body_chunk: false,
      })
      .unwrap(),
    )
    .unwrap();

    // Simulate receiving no data
    let mut buffer = [0u8; 128];
    let read_bytes = reader.read(&mut buffer).unwrap();
    assert_eq!(read_bytes, 0);
  }

  #[test]
  fn test_multiple_chunks() {
    let (tx_inner, _rx_outer) = interprocess::unnamed_pipe::pipe().unwrap();
    let (mut tx_outer, rx_inner) = interprocess::unnamed_pipe::pipe().unwrap();
    let mut reader = WsgidBodyReader::new(Arc::new(Mutex::new(tx_inner)), Arc::new(Mutex::new(rx_inner)));

    let input1 = b"First chunk ";
    let input2 = b"Second chunk";

    for chunk in &[input1, input2] {
      write_ipc_message(
        &mut tx_outer,
        &postcard::to_allocvec(&ServerToProcessPoolMessage {
          application_id: None,
          environment_variables: None,
          body_chunk: Some(chunk.to_vec()),
          body_error_message: None,
          requests_body_chunk: false,
        })
        .unwrap(),
      )
      .unwrap();
    }

    write_ipc_message(
      &mut tx_outer,
      &postcard::to_allocvec(&ServerToProcessPoolMessage {
        application_id: None,
        environment_variables: None,
        body_chunk: None,
        body_error_message: None,
        requests_body_chunk: false,
      })
      .unwrap(),
    )
    .unwrap();

    let mut buffer = [0u8; 64];
    let mut total_read = 0;
    loop {
      let bytes_read = reader.read(&mut buffer[total_read..]).unwrap();
      if bytes_read == 0 {
        break;
      }
      total_read += bytes_read;
    }

    let expected = [input1.to_owned(), input2.to_owned()].concat();
    assert_eq!(&buffer[..total_read], &expected[..]);
  }

  #[test]
  fn test_error_message() {
    let (tx_inner, _rx_outer) = interprocess::unnamed_pipe::pipe().unwrap();
    let (mut tx_outer, rx_inner) = interprocess::unnamed_pipe::pipe().unwrap();
    let mut reader = WsgidBodyReader::new(Arc::new(Mutex::new(tx_inner)), Arc::new(Mutex::new(rx_inner)));

    let error_message = "something went wrong".to_string();
    write_ipc_message(
      &mut tx_outer,
      &postcard::to_allocvec(&ServerToProcessPoolMessage {
        application_id: None,
        environment_variables: None,
        body_chunk: None,
        body_error_message: Some(error_message.clone()),
        requests_body_chunk: false,
      })
      .unwrap(),
    )
    .unwrap();

    let mut buffer = [0u8; 128];
    let result = reader.read(&mut buffer);

    assert!(result.is_err());
    assert_eq!(result.unwrap_err().to_string(), error_message);
  }

  #[test]
  fn test_buffering_behavior() {
    let (tx_inner, _rx_outer) = interprocess::unnamed_pipe::pipe().unwrap();
    let (mut tx_outer, rx_inner) = interprocess::unnamed_pipe::pipe().unwrap();
    let mut reader = WsgidBodyReader::new(Arc::new(Mutex::new(tx_inner)), Arc::new(Mutex::new(rx_inner)));

    let data = b"This is a long chunk of data";
    write_ipc_message(
      &mut tx_outer,
      &postcard::to_allocvec(&ServerToProcessPoolMessage {
        application_id: None,
        environment_variables: None,
        body_chunk: Some(data.to_vec()),
        body_error_message: None,
        requests_body_chunk: false,
      })
      .unwrap(),
    )
    .unwrap();
    write_ipc_message(
      &mut tx_outer,
      &postcard::to_allocvec(&ServerToProcessPoolMessage {
        application_id: None,
        environment_variables: None,
        body_chunk: None,
        body_error_message: None,
        requests_body_chunk: false,
      })
      .unwrap(),
    )
    .unwrap();

    let mut buf1 = [0u8; 10];
    let mut buf2 = [0u8; 64];

    let n1 = reader.read(&mut buf1).unwrap();
    assert_eq!(&buf1[..n1], &data[..n1]);

    let n2 = reader.read(&mut buf2).unwrap();
    let expected_remainder = &data[n1..];
    assert_eq!(&buf2[..n2], expected_remainder);
  }
}
