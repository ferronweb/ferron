pub fn construct_fastcgi_record(record_type: u8, request_id: u16, content: &[u8]) -> Vec<u8> {
  let mut record = Vec::new();

  // FastCGI version: FCGI_VERSION_1
  record.push(1);

  // Record type
  record.extend_from_slice(&(record_type.to_be_bytes()));

  // Request ID
  record.extend_from_slice(&(request_id.to_be_bytes()));

  // Content length
  let content_length = content.len() as u16;
  record.extend_from_slice(&(content_length.to_be_bytes()));

  // Padding length
  let content_length_modulo = (content_length % 8) as u8;
  let padding_length = match content_length_modulo {
    0 => 0,
    _ => 8 - content_length_modulo,
  };
  record.extend_from_slice(&(padding_length.to_be_bytes()));

  // Reserved
  record.push(0);

  // Content
  record.extend_from_slice(content);

  // Padding
  record.append(&mut vec![0u8; padding_length as usize]);

  record
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_construct_fastcgi_record() {
    // Test case 1: Empty content
    let record_type = 1;
    let request_id = 1234;
    let content: &[u8] = &[];
    let expected_record = vec![
      1, // FastCGI version
      1, // Record type
      4, 210, // Request ID
      0, 0, // Content length
      0, // Padding length
      0, // Reserved
    ];
    assert_eq!(
      construct_fastcgi_record(record_type, request_id, content),
      expected_record
    );

    // Test case 2: Content with length 5
    let record_type = 2;
    let request_id = 5678;
    let content = b"Hello";
    let expected_record = vec![
      1, // FastCGI version
      2, // Record type
      22, 46, // Request ID
      0, 5, // Content length
      3, // Padding length
      0, // Reserved
      72, 101, 108, 108, 111, // Content
      0, 0, 0, // Padding
    ];
    assert_eq!(
      construct_fastcgi_record(record_type, request_id, content),
      expected_record
    );

    // Test case 3: Content with length 8 (no padding needed)
    let record_type = 3;
    let request_id = 9012;
    let content = b"12345678";
    let expected_record = vec![
      1, // FastCGI version
      3, // Record type
      35, 52, // Request ID
      0, 8, // Content length
      0, // Padding length
      0, // Reserved
      49, 50, 51, 52, 53, 54, 55, 56, // Content
    ];
    assert_eq!(
      construct_fastcgi_record(record_type, request_id, content),
      expected_record
    );
  }
}
