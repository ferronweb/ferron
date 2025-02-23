// Copyright (c) 2018-2025 SVR.JS
// Portions of this file are derived from SVR.JS (https://git.svrjs.org/svrjs/svrjs).
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
use std::str;

pub fn sanitize_url(resource: &str, allow_double_slashes: bool) -> Result<String> {
  if resource == "*" || resource.is_empty() {
    return Ok(resource.to_string());
  }

  let mut sanitized = String::with_capacity(resource.len());

  // Remove null bytes and handle initial sanitization
  for &ch in resource.as_bytes() {
    if ch != b'\0' {
      sanitized.push(ch as char);
    }
  }

  // Check for malformed URL encoding (invalid percent encoding)
  let bytes = sanitized.as_bytes();
  let mut i = 0;
  while i < bytes.len() {
    if bytes[i] == b'%' {
      if i + 2 >= bytes.len() {
        return Err(anyhow!("URI malformed"));
      }
      let hex = &bytes[i + 1..i + 3];
      if !hex[0].is_ascii_hexdigit() || !hex[1].is_ascii_hexdigit() {
        return Err(anyhow!("URI malformed"));
      }
      let value = u8::from_str_radix(str::from_utf8(hex)?, 16)?;
      if value == 0xc0 || value == 0xc1 || value >= 0xfe {
        return Err(anyhow!("URI malformed"));
      }
    }
    i += 1;
  }

  // Decode percent-encoded characters while preserving safe ones
  let mut decoded = String::with_capacity(sanitized.len());
  let bytes = sanitized.as_bytes();
  let mut i = 0;
  while i < bytes.len() {
    if bytes[i] == b'%' && i + 2 < bytes.len() {
      let hex = &bytes[i + 1..i + 3];
      if let Ok(value) = u8::from_str_radix(str::from_utf8(hex)?, 16) {
        if value != 0 {
          let decoded_char = value as char;
          if decoded_char.is_ascii_alphanumeric()
                        || "!$&'()*+,-./0123456789:;=@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]_abcdefghijklmnopqrstuvwxyz~"
                            .contains(decoded_char)
                    {
                        decoded.push(decoded_char);
                    } else {
                        decoded.push('%');
                        decoded.push(hex[0] as char);
                        decoded.push(hex[1] as char);
                    }
          i += 2;
        } else {
          i += 3;
          continue;
        }
      } else {
        decoded.push('%');
      }
    } else {
      decoded.push(bytes[i] as char);
    }
    i += 1;
  }

  // Encode unsafe characters
  let mut encoded = String::with_capacity(decoded.len());
  for ch in decoded.chars() {
    match ch {
      '<' | '>' | '^' | '`' | '{' | '|' | '}' => {
        encoded.push_str(&format!("%{:02X}", ch as u8));
      }
      _ => encoded.push(ch),
    }
  }

  // Ensure the resource starts with a slash
  if !encoded.starts_with('/') {
    encoded.insert(0, '/');
  }

  // Convert backslashes to slashes and handle duplicate slashes
  let mut final_resource = String::with_capacity(encoded.len());
  let mut last_was_slash = false;
  for ch in encoded.chars() {
    if ch == '\\' {
      final_resource.push('/');
      last_was_slash = true;
    } else if ch == '/' {
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

  // Normalize path segments (remove ".", "..", trailing dots)
  let mut segments: Vec<&str> = Vec::new();
  for mut part in final_resource.split('/') {
    match part {
      "." => continue,
      ".." => {
        segments.pop(); // Go up one directory
      }
      "" => {
        if allow_double_slashes {
          segments.push("");
        }
      }
      _ => {
        while part.ends_with('.') {
          part = &part[..part.len() - 1];
        }
        if !part.is_empty() {
          segments.push(part);
        }
      }
    }
  }

  final_resource = if allow_double_slashes {
    segments.join("/")
  } else if !segments.is_empty() && final_resource.ends_with('/') {
    format!("/{}/", segments.join("/"))
  } else {
    format!("/{}", segments.join("/"))
  };

  // Remove any remaining "/../" sequences
  while final_resource.contains("/../") {
    final_resource = final_resource.replacen("/../", "", 1);
  }

  // Ensure result is not empty
  if final_resource.is_empty() {
    final_resource.push('/');
  }

  Ok(final_resource)
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
