use hyper::body::{Buf, Bytes};
use smallvec::{SmallVec, ToSmallVec};
use tokio_util::bytes::BytesMut;
use tokio_util::codec::Decoder;

/// Decoded FastCGI data
#[derive(Debug)]
pub enum FcgiDecodedData {
  /// Standard output
  Stdout(Bytes),

  /// Standard error
  Stderr(Bytes),
}

enum FcgiDecodeState {
  ReadingHead,
  ReadingContent,
  Finished,
}

/// Encoder that decodes from FastCGI records
pub struct FcgiDecoder {
  header: SmallVec<[u8; 8]>,
  content_length: u16,
  padding_length: u8,
  state: FcgiDecodeState,
}

impl FcgiDecoder {
  /// Creates a new FastCGI decoder
  pub fn new() -> Self {
    Self {
      header: SmallVec::new(),
      content_length: 0,
      padding_length: 0,
      state: FcgiDecodeState::ReadingHead,
    }
  }
}

impl Decoder for FcgiDecoder {
  type Error = std::io::Error;
  type Item = FcgiDecodedData;

  fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
    loop {
      match self.state {
        FcgiDecodeState::ReadingHead => {
          if src.len() >= 8 {
            let header = &src[..8];
            self.header = header.to_smallvec();
            src.advance(8);
            self.content_length = u16::from_be_bytes(
              self.header[4..6]
                .try_into()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?,
            );
            self.padding_length = self.header[6];
            self.state = FcgiDecodeState::ReadingContent;
          } else {
            return Ok(None);
          }
        }
        FcgiDecodeState::ReadingContent => {
          if src.len() >= self.content_length as usize + self.padding_length as usize {
            let request_id = u16::from_be_bytes(
              self.header[2..4]
                .try_into()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?,
            );
            let record_type = self.header[1];
            if request_id != 1 || (record_type != 3 && record_type != 6 && record_type != 7) {
              // Ignore the record for wrong request ID or if the record isn't END_REQUEST, STDOUT or STDERR
              src.advance(self.content_length as usize + self.padding_length as usize);
              return Ok(None);
            }
            let content_borrowed = &src[..(self.content_length as usize)];
            let content = content_borrowed.to_vec();
            src.advance(self.content_length as usize + self.padding_length as usize);

            match record_type {
              3 => {
                // END_REQUEST record
                if content.len() > 5 {
                  let app_status = u32::from_be_bytes(
                    content[0..4]
                      .try_into()
                      .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?,
                  );
                  let protocol_status = content[4];
                  match protocol_status {
                    0 => (),
                    1 => return Err(std::io::Error::other("FastCGI server overloaded")),
                    2 => return Err(std::io::Error::other("Role not supported by the FastCGI application")),
                    3 => {
                      return Err(std::io::Error::other(
                        "Multiplexed connections not supported by the FastCGI application",
                      ))
                    }
                    _ => return Err(std::io::Error::other("Unknown error")),
                  }

                  self.state = FcgiDecodeState::Finished;
                  if app_status != 0 {
                    // Inject data into standard error stream
                    return Ok(Some(FcgiDecodedData::Stderr(Bytes::from_owner(format!(
                      "FastCGI application exited with code {app_status}"
                    )))));
                  }
                } else {
                  // Record malformed, ignoring the record
                  return Ok(None);
                }
              }
              6 => {
                // STDOUT record
                self.state = FcgiDecodeState::ReadingHead;
                return Ok(Some(FcgiDecodedData::Stdout(Bytes::from_owner(content))));
              }
              7 => {
                // STDERR record
                self.state = FcgiDecodeState::ReadingHead;
                if content.is_empty() {
                  continue;
                }
                return Ok(Some(FcgiDecodedData::Stderr(Bytes::from_owner(content))));
              }
              _ => {
                // This should be unreachable
                unreachable!()
              }
            };
          } else {
            return Ok(None);
          }
        }
        FcgiDecodeState::Finished => {
          src.clear();
          return Ok(None);
        }
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::util::fcgi::construct_fastcgi_record;
  use tokio_util::bytes::BytesMut;
  use tokio_util::codec::Decoder;

  #[test]
  fn test_fcgi_decoder_stdout() {
    let mut decoder = FcgiDecoder::new();
    let mut buf = BytesMut::new();

    // Construct a STDOUT record
    let record_type = 6;
    let request_id = 1;
    let content = b"Hello, FastCGI!";
    let record = construct_fastcgi_record(record_type, request_id, content);

    buf.extend_from_slice(&record);

    let result = decoder.decode(&mut buf).unwrap();
    assert!(result.is_some());
    if let Some(FcgiDecodedData::Stdout(data)) = result {
      assert_eq!(&data[..], content);
    } else {
      panic!("Expected STDOUT data");
    }
  }

  #[test]
  fn test_fcgi_decoder_stderr() {
    let mut decoder = FcgiDecoder::new();
    let mut buf = BytesMut::new();

    // Construct a STDERR record
    let record_type = 7;
    let request_id = 1;
    let content = b"Error message";
    let record = construct_fastcgi_record(record_type, request_id, content);

    buf.extend_from_slice(&record);

    let result = decoder.decode(&mut buf).unwrap();
    assert!(result.is_some());
    if let Some(FcgiDecodedData::Stderr(data)) = result {
      assert_eq!(&data[..], content);
    } else {
      panic!("Expected STDERR data");
    }
  }

  #[test]
  fn test_fcgi_decoder_end_request() {
    let mut decoder = FcgiDecoder::new();
    let mut buf = BytesMut::new();

    // Construct an END_REQUEST record
    let record_type = 3;
    let request_id = 1;
    let mut content = [0u8; 4].to_vec(); // App status
    content.push(0); // Protocol status
    let record = construct_fastcgi_record(record_type, request_id, &content);

    buf.extend_from_slice(&record);

    let result = decoder.decode(&mut buf).unwrap();
    assert!(result.is_none()); // No data for END_REQUEST
  }

  #[test]
  fn test_fcgi_decoder_invalid_record() {
    let mut decoder = FcgiDecoder::new();
    let mut buf = BytesMut::new();

    // Construct an invalid record with wrong request ID
    let record_type = 6;
    let request_id = 2; // Invalid request ID
    let content = b"Invalid record";
    let record = construct_fastcgi_record(record_type, request_id, content);

    buf.extend_from_slice(&record);

    let result = decoder.decode(&mut buf).unwrap();
    assert!(result.is_none()); // Invalid record should be ignored
  }
}
