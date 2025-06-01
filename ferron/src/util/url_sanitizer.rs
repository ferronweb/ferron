// Copyright (c) 2018-2025 SVR.JS
// Portions of this file are derived from SVR.JS (https://github.com/svr-js/svrjs).
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
//
use anyhow::{anyhow, Result};
use smallvec::SmallVec;

// Lookup table for safe characters that don't need encoding
static SAFE_CHARS: [bool; 256] = {
  let mut table = [false; 256];
  let safe_bytes =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789!$&'()*+,-./:;=@[]_~";
  let mut i = 0;
  while i < safe_bytes.len() {
    table[safe_bytes[i] as usize] = true;
    i += 1;
  }
  table
};

// Hex lookup table for faster encoding
static HEX_CHARS: [u8; 16] = *b"0123456789ABCDEF";

/// Sanitizes the URL
pub fn sanitize_url(resource: &str, allow_double_slashes: bool) -> Result<String> {
  if resource == "*" || resource.is_empty() {
    return Ok(resource.to_string());
  }

  let bytes = resource.as_bytes();
  let mut result = SmallVec::<[u8; 256]>::with_capacity(bytes.len() * 2);

  // Combined pass: remove nulls, validate percent encoding, decode/encode in one go
  let mut i = 0;
  while i < bytes.len() {
    let byte = bytes[i];

    // Skip null bytes
    if byte == 0 {
      i += 1;
      continue;
    }

    if byte == b'%' {
      // Validate percent encoding
      if i + 2 >= bytes.len() {
        return Err(anyhow!("URI malformed"));
      }

      let hi = bytes[i + 1];
      let lo = bytes[i + 2];

      if !hi.is_ascii_hexdigit() || !lo.is_ascii_hexdigit() {
        return Err(anyhow!("URI malformed"));
      }

      let value = hex_to_byte_fast(hi, lo)?;
      if value == 0xc0 || value == 0xc1 || value >= 0xfe {
        return Err(anyhow!("URI malformed"));
      }

      // Decode if safe, otherwise keep encoded
      if value == 0 {
        // Skip null bytes even when percent-encoded
        i += 3;
        continue;
      } else if SAFE_CHARS[value as usize] {
        result.push(value);
      } else {
        result.push(b'%');
        result.push(hi);
        result.push(lo);
      }
      i += 3;
    } else {
      // Handle special characters that need encoding
      match byte {
        b'<' | b'>' | b'^' | b'`' | b'{' | b'|' | b'}' => {
          result.push(b'%');
          result.push(HEX_CHARS[(byte >> 4) as usize]);
          result.push(HEX_CHARS[(byte & 0xF) as usize]);
        }
        _ => result.push(byte),
      }
      i += 1;
    }
  }

  // Ensure starts with '/'
  if result.is_empty() || result[0] != b'/' {
    result.insert(0, b'/');
  }

  // Normalize slashes and build segments in one pass
  let mut segments = SmallVec::<[SmallVec<[u8; 32]>; 16]>::new();
  let mut current_segment = SmallVec::<[u8; 32]>::new();
  let mut last_was_slash = true; // Start with true since we ensured it starts with '/'

  i = 1; // Skip the initial '/'
  while i < result.len() {
    let byte = result[i];

    if byte == b'\\' || byte == b'/' {
      if !current_segment.is_empty() {
        // Trim trailing dots, but preserve "." and ".." for navigation
        if current_segment.as_slice() != b"." && current_segment.as_slice() != b".." {
          while let Some(&b'.') = current_segment.last() {
            current_segment.pop();
          }
        }

        if !current_segment.is_empty() {
          segments.push(current_segment);
          current_segment = SmallVec::new();
        }
      }

      if allow_double_slashes && last_was_slash {
        // Add empty segment for double slash
        segments.push(SmallVec::new());
      }
      last_was_slash = true;
    } else {
      current_segment.push(byte);
      last_was_slash = false;
    }
    i += 1;
  }

  // Handle final segment
  if !current_segment.is_empty() {
    // Trim trailing dots, but preserve "." and ".." for navigation
    if current_segment.as_slice() != b"." && current_segment.as_slice() != b".." {
      while let Some(&b'.') = current_segment.last() {
        current_segment.pop();
      }
    }
    if !current_segment.is_empty() {
      segments.push(current_segment);
    }
  }

  // Process segments for . and .. navigation
  let mut final_segments = SmallVec::<[SmallVec<[u8; 32]>; 16]>::new();
  for segment in segments {
    if segment.is_empty() && allow_double_slashes {
      final_segments.push(segment);
    } else if segment.as_slice() == b"." {
      // Skip current directory
      continue;
    } else if segment.as_slice() == b".." {
      // Parent directory - remove last segment
      final_segments.pop();
    } else if !segment.is_empty() {
      final_segments.push(segment);
    }
  }

  // Build final result
  let mut final_result = SmallVec::<[u8; 256]>::with_capacity(result.len());
  final_result.push(b'/');

  let preserve_trailing_slash =
    !result.is_empty() && (result[result.len() - 1] == b'/' || result[result.len() - 1] == b'\\');

  for (idx, segment) in final_segments.iter().enumerate() {
    if idx > 0 || (allow_double_slashes && segment.is_empty()) {
      final_result.push(b'/');
    }
    final_result.extend_from_slice(segment);
  }

  // Add trailing slash if it was originally present and we're not at the root directory
  // or if we are at root but the original path was just "/"
  // if preserve_trailing_slash
  //   && (!final_segments.is_empty() || (final_segments.is_empty() && result.len() == 1))
  // {
  //   final_result.push(b'/');
  // }

  // Add trailing slash if it was originally present and we have segments,
  // but don't add it if we're just dealing with the root "/"
  if preserve_trailing_slash && !final_segments.is_empty() {
    final_result.push(b'/');
  }

  // Convert to string
  String::from_utf8(final_result.into_vec()).map_err(|_| anyhow!("Invalid UTF-8 in result"))
}

#[inline(always)]
fn hex_to_byte_fast(hi: u8, lo: u8) -> Result<u8> {
  #[inline(always)]
  fn hex_val(c: u8) -> Option<u8> {
    match c {
      b'0'..=b'9' => Some(c - b'0'),
      b'a'..=b'f' => Some(10 + (c - b'a')),
      b'A'..=b'F' => Some(10 + (c - b'A')),
      _ => None,
    }
  }
  match (hex_val(hi), hex_val(lo)) {
    (Some(h), Some(l)) => Ok(h << 4 | l),
    _ => Err(anyhow!("Invalid hex")),
  }
}

// Path sanitizer tests taken from SVR.JS web server
#[cfg(test)]
mod tests {
  use super::*;
  use anyhow::Result;

  #[test]
  fn should_not_change_slash() -> Result<()> {
    assert_eq!(sanitize_url("/", false)?, "/");
    Ok(())
  }

  #[test]
  fn should_return_asterisk_for_asterisk() -> Result<()> {
    assert_eq!(sanitize_url("*", false)?, "*");
    Ok(())
  }

  #[test]
  fn should_return_empty_string_for_empty_string() -> Result<()> {
    assert_eq!(sanitize_url("", false)?, "");
    Ok(())
  }

  #[test]
  fn should_remove_null_characters() -> Result<()> {
    assert_eq!(sanitize_url("/test%00", false)?, "/test");
    assert_eq!(sanitize_url("/test\0", false)?, "/test");
    Ok(())
  }

  #[test]
  fn should_throw_uri_error_for_malformed_url() {
    assert!(sanitize_url("%c0%af", false).is_err());
    assert!(sanitize_url("%u002f", false).is_err());
    assert!(sanitize_url("%as", false).is_err());
  }

  #[test]
  fn should_ensure_the_resource_starts_with_a_slash() -> Result<()> {
    assert_eq!(sanitize_url("test", false)?, "/test");
    Ok(())
  }

  #[test]
  fn should_convert_backslashes_to_slashes() -> Result<()> {
    assert_eq!(sanitize_url("test\\path", false)?, "/test/path");
    Ok(())
  }

  #[test]
  fn should_handle_duplicate_slashes() -> Result<()> {
    assert_eq!(sanitize_url("test//path", false)?, "/test/path");
    assert_eq!(sanitize_url("test//path", true)?, "/test//path");
    Ok(())
  }

  #[test]
  fn should_handle_relative_navigation() -> Result<()> {
    assert_eq!(sanitize_url("/./test", false)?, "/test");
    assert_eq!(sanitize_url("/../test", false)?, "/test");
    assert_eq!(sanitize_url("../test", false)?, "/test");
    assert_eq!(sanitize_url("./test", false)?, "/test");
    assert_eq!(sanitize_url("/test/./", false)?, "/test/");
    assert_eq!(sanitize_url("/test/../", false)?, "/");
    assert_eq!(sanitize_url("/test/../path", false)?, "/path");
    Ok(())
  }

  #[test]
  fn should_remove_trailing_dots_in_paths() -> Result<()> {
    assert_eq!(sanitize_url("/test...", false)?, "/test");
    assert_eq!(sanitize_url("/test.../", false)?, "/test/");
    Ok(())
  }

  #[test]
  fn should_return_slash_for_empty_sanitized_resource() -> Result<()> {
    assert_eq!(sanitize_url("/../..", false)?, "/");
    Ok(())
  }

  #[test]
  fn should_encode_special_characters() -> Result<()> {
    assert_eq!(sanitize_url("/test<path>", false)?, "/test%3Cpath%3E");
    assert_eq!(sanitize_url("/test^path", false)?, "/test%5Epath");
    assert_eq!(sanitize_url("/test`path", false)?, "/test%60path");
    assert_eq!(sanitize_url("/test{path}", false)?, "/test%7Bpath%7D");
    assert_eq!(sanitize_url("/test|path", false)?, "/test%7Cpath");
    Ok(())
  }

  #[test]
  fn should_preserve_certain_characters() -> Result<()> {
    assert_eq!(sanitize_url("/test!path", false)?, "/test!path");
    assert_eq!(sanitize_url("/test$path", false)?, "/test$path");
    assert_eq!(sanitize_url("/test&path", false)?, "/test&path");
    assert_eq!(sanitize_url("/test-path", false)?, "/test-path");
    assert_eq!(sanitize_url("/test=path", false)?, "/test=path");
    assert_eq!(sanitize_url("/test@path", false)?, "/test@path");
    assert_eq!(sanitize_url("/test_path", false)?, "/test_path");
    assert_eq!(sanitize_url("/test~path", false)?, "/test~path");
    Ok(())
  }

  #[test]
  fn should_decode_url_encoded_characters_while_preserving_certain_characters() -> Result<()> {
    assert_eq!(sanitize_url("/test%20path", false)?, "/test%20path");
    assert_eq!(sanitize_url("/test%21path", false)?, "/test!path");
    assert_eq!(sanitize_url("/test%22path", false)?, "/test%22path");
    assert_eq!(sanitize_url("/test%24path", false)?, "/test$path");
    assert_eq!(sanitize_url("/test%25path", false)?, "/test%25path");
    assert_eq!(sanitize_url("/test%26path", false)?, "/test&path");
    assert_eq!(sanitize_url("/test%2Dpath", false)?, "/test-path");
    assert_eq!(sanitize_url("/test%3Cpath", false)?, "/test%3Cpath");
    assert_eq!(sanitize_url("/test%3Dpath", false)?, "/test=path");
    assert_eq!(sanitize_url("/test%3Epath", false)?, "/test%3Epath");
    assert_eq!(sanitize_url("/test%40path", false)?, "/test@path");
    assert_eq!(sanitize_url("/test%5Fpath", false)?, "/test_path");
    assert_eq!(sanitize_url("/test%7Dpath", false)?, "/test%7Dpath");
    assert_eq!(sanitize_url("/test%7Epath", false)?, "/test~path");
    Ok(())
  }

  #[test]
  fn should_decode_url_encoded_alphanumeric_characters_while_preserving_certain_characters(
  ) -> Result<()> {
    assert_eq!(sanitize_url("/conf%69g.json", false)?, "/config.json");
    assert_eq!(sanitize_url("/CONF%49G.JSON", false)?, "/CONFIG.JSON");
    assert_eq!(sanitize_url("/svr%32.js", false)?, "/svr2.js");
    assert_eq!(sanitize_url("/%73%76%72%32%2E%6A%73", false)?, "/svr2.js");
    Ok(())
  }

  #[test]
  fn should_decode_url_encoded_characters_regardless_of_the_letter_case_of_the_url_encoding(
  ) -> Result<()> {
    assert_eq!(sanitize_url("/%5f", false)?, "/_");
    assert_eq!(sanitize_url("/%5F", false)?, "/_");
    Ok(())
  }
}
