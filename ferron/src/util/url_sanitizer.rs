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

/// Sanitizes the URL
pub fn sanitize_url(resource: &str, allow_double_slashes: bool) -> Result<String> {
  if resource == "*" || resource.is_empty() {
    return Ok(resource.to_string());
  }

  // Remove null bytes directly without allocating a new string unless needed
  let sanitized: Vec<u8> = resource
    .as_bytes()
    .iter()
    .cloned()
    .filter(|&b| b != 0)
    .collect();

  // Check for malformed percent encoding
  let mut i = 0;
  while i < sanitized.len() {
    if sanitized[i] == b'%' {
      if i + 2 >= sanitized.len() {
        return Err(anyhow!("URI malformed"));
      }
      let hi = sanitized[i + 1];
      let lo = sanitized[i + 2];
      if !hi.is_ascii_hexdigit() || !lo.is_ascii_hexdigit() {
        return Err(anyhow!("URI malformed"));
      }
      let value = hex_to_byte(hi, lo)?;
      if value == 0xc0 || value == 0xc1 || value >= 0xfe {
        return Err(anyhow!("URI malformed"));
      }
    }
    i += 1;
  }

  // Decode percent-encoded characters with allowed safe chars
  let mut decoded = String::with_capacity(sanitized.len());
  let mut i = 0;
  while i < sanitized.len() {
    if sanitized[i] == b'%' && i + 2 < sanitized.len() {
      let val = hex_to_byte(sanitized[i + 1], sanitized[i + 2])?;
      if val != 0 {
        let ch = val as char;
        if ch.is_ascii_alphanumeric()
          || matches!(ch, '!' | '$' | '&' | '\'' | '(' | ')' | '*' | '+' | ',' | '-' | '.' | '/'
                        | '0'..='9' | ':' | ';' | '=' | '@' | 'A'..='Z' | '[' | '\\' | ']' | '_' | 'a'..='z' | '~')
        {
          decoded.push(ch);
        } else {
          decoded.push('%');
          decoded.push(sanitized[i + 1] as char);
          decoded.push(sanitized[i + 2] as char);
        }
        i += 3;
        continue;
      } else {
        i += 3;
        continue;
      }
    } else {
      decoded.push(sanitized[i] as char);
      i += 1;
    }
  }

  // Encode unsafe characters
  let mut encoded = String::with_capacity(decoded.len());
  for ch in decoded.chars() {
    match ch {
      '<' | '>' | '^' | '`' | '{' | '|' | '}' => {
        encoded.push('%');
        encoded.push_str(&format!("{:02X}", ch as u8));
      }
      _ => encoded.push(ch),
    }
  }

  // Ensure starts with '/'
  if !encoded.starts_with('/') {
    encoded.insert(0, '/');
  }

  // Normalize slashes
  let mut final_resource = String::with_capacity(encoded.len());
  let mut last_was_slash = false;
  for ch in encoded.chars() {
    if ch == '\\' || ch == '/' {
      if !allow_double_slashes && last_was_slash {
        continue;
      }
      final_resource.push('/');
      last_was_slash = true;
    } else {
      final_resource.push(ch);
      last_was_slash = false;
    }
  }

  // Normalize path segments
  let mut segments: SmallVec<[&str; 16]> = SmallVec::new();
  for part in final_resource.split('/') {
    match part {
      "" if allow_double_slashes => segments.push(""),
      "." => continue,
      ".." => {
        segments.pop();
      }
      _ => {
        let trimmed = part.trim_end_matches('.');
        if !trimmed.is_empty() {
          segments.push(trimmed);
        }
      }
    }
  }

  let mut final_path = if allow_double_slashes {
    segments.join("/")
  } else if !segments.is_empty() && final_resource.ends_with('/') {
    format!("/{}/", segments.join("/"))
  } else {
    format!("/{}", segments.join("/"))
  };

  // Remove remaining '/../' sequences
  while final_path.contains("/../") {
    final_path = final_path.replace("/../", "");
  }

  // Ensure non-empty result
  if final_path.is_empty() {
    final_path.push('/');
  }

  Ok(final_path)
}

#[inline(always)]
fn hex_to_byte(hi: u8, lo: u8) -> Result<u8> {
  fn val(c: u8) -> Option<u8> {
    match c {
      b'0'..=b'9' => Some(c - b'0'),
      b'a'..=b'f' => Some(10 + (c - b'a')),
      b'A'..=b'F' => Some(10 + (c - b'A')),
      _ => None,
    }
  }
  match (val(hi), val(lo)) {
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
