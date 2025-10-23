/// Constructs a FastCGI name-value pair
pub fn construct_fastcgi_name_value_pair(name: &[u8], value: &[u8]) -> Vec<u8> {
  // Name and value lengths (for determining the vector allocation size)
  let name_length = name.len();
  let value_length = value.len();

  // Allocate the vector with the preallocated capacity
  let mut name_value_pair = Vec::with_capacity(
    if name_length < 128 { 1 } else { 4 } + if value_length < 128 { 1 } else { 4 } + name_length + value_length,
  );

  // Name length
  if name_length < 128 {
    name_value_pair.extend_from_slice(&(name_length as u8).to_be_bytes());
  } else {
    name_value_pair.extend_from_slice(&((name_length as u32) | 0x80000000).to_be_bytes());
  }

  // Value length
  if value_length < 128 {
    name_value_pair.extend_from_slice(&(value_length as u8).to_be_bytes());
  } else {
    name_value_pair.extend_from_slice(&((value_length as u32) | 0x80000000).to_be_bytes());
  }

  // Name
  name_value_pair.extend_from_slice(name);

  // Value
  name_value_pair.extend_from_slice(value);

  name_value_pair
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_short_name_and_value() {
    let name = b"HOST";
    let value = b"localhost";
    let expected = vec![
      0x04, // Name length (4 bytes)
      0x09, // Value length (9 bytes)
      b'H', b'O', b'S', b'T', // Name
      b'l', b'o', b'c', b'a', b'l', b'h', b'o', b's', b't', // Value
    ];
    assert_eq!(construct_fastcgi_name_value_pair(name, value), expected);
  }

  #[test]
  fn test_long_name_and_value() {
    let name = vec![b'N'; 130]; // Name length 130
    let value = vec![b'V'; 135]; // Value length 135
    let mut expected = vec![
      0x80, 0x00, 0x00, 0x82, // Name length (130 bytes)
      0x80, 0x00, 0x00, 0x87, // Value length (135 bytes)
    ];
    expected.extend_from_slice(&name);
    expected.extend_from_slice(&value);
    assert_eq!(construct_fastcgi_name_value_pair(&name, &value), expected);
  }

  #[test]
  fn test_empty_name_and_value() {
    let name = b"";
    let value = b"";
    let expected = vec![
      0x00, // Name length (0 bytes)
      0x00, // Value length (0 bytes)
    ];
    assert_eq!(construct_fastcgi_name_value_pair(name, value), expected);
  }

  #[test]
  fn test_name_length_127() {
    let name = vec![b'a'; 127];
    let value = b"value";
    let mut expected = vec![
      0x7f, // Name length (127 bytes)
      0x05, // Value length (5 bytes)
    ];
    expected.extend_from_slice(&name);
    expected.extend_from_slice(value);
    assert_eq!(construct_fastcgi_name_value_pair(&name, value), expected);
  }

  #[test]
  fn test_value_length_127() {
    let name = b"name";
    let value = vec![b'b'; 127];
    let mut expected = vec![
      0x04, // Name length (4 bytes)
      0x7f, // Value length (127 bytes)
    ];
    expected.extend_from_slice(name);
    expected.extend_from_slice(&value);
    assert_eq!(construct_fastcgi_name_value_pair(name, &value), expected);
  }
}
