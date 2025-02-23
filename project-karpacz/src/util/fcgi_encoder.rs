use tokio_util::bytes::{BufMut, BytesMut};
use tokio_util::codec::Encoder;

use crate::project_karpacz_util::fcgi_record::construct_fastcgi_record;

pub struct FcgiEncoder;

impl FcgiEncoder {
  pub fn new() -> Self {
    FcgiEncoder
  }
}

impl Encoder<&[u8]> for FcgiEncoder {
  type Error = std::io::Error;

  fn encode(&mut self, item: &[u8], dst: &mut BytesMut) -> Result<(), Self::Error> {
    let mut offset = 0;
    while offset < item.len() {
      let chunk_size = std::cmp::min(65536, item.len() - offset);
      let chunk = &item[offset..offset + chunk_size];

      // Record type 5 means STDIN
      let record = construct_fastcgi_record(5, 1, chunk);
      dst.put(record.as_slice());

      offset += chunk_size;
    }

    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use tokio_util::codec::Encoder;

  #[test]
  fn test_fcgi_encoder() {
    let mut encoder = FcgiEncoder::new();
    let mut dst = BytesMut::new();
    let item = b"Test data";

    encoder.encode(item, &mut dst).unwrap();

    // Expected encoded record structure
    let expected_record = vec![
      1, // FCGI_VERSION_1
      5, // Record type
      0, 1, // Request ID (big-endian)
      0, 9, // Content length (big-endian)
      7, // Padding length
      0, // Reserved
      84, 101, 115, 116, 32, // Content: "Test "
      100, 97, 116, 97, // Content: "data"
      0, 0, 0, 0, 0, 0, 0, // Padding
    ];

    assert_eq!(dst.to_vec(), expected_record);
  }
}
