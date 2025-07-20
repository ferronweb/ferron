// Sizify function taken from SVR.JS and rewritten from JavaScript to Rust
// SVR.JS is licensed under MIT, so below is the copyright notice:
//
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

/// Converts the file size into a human-readable one
pub fn sizify(bytes: u64, add_i: bool) -> String {
  if bytes == 0 {
    return "0".to_string();
  }

  let prefixes = ["", "K", "M", "G", "T", "P", "E", "Z", "Y", "R", "Q"];
  let prefix_index = ((bytes as f64).log2() / 10.0).floor().min(prefixes.len() as f64 - 1.0) as usize;
  let prefix_index_translated = 2_i64.pow(10 * prefix_index as u32);
  let decimal_points = ((2.0 - (bytes as f64 / prefix_index_translated as f64).log10().floor()) as i32).max(0);

  let size = ((bytes as f64 / prefix_index_translated as f64) * 10_f64.powi(decimal_points)).ceil()
    / 10_f64.powi(decimal_points);
  let prefix = prefixes[prefix_index];
  let suffix = if prefix_index > 0 && add_i { "i" } else { "" };

  format!("{size}{prefix}{suffix}")
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_sizify_zero_bytes() {
    assert_eq!(sizify(0, false), "0");
  }

  #[test]
  fn test_sizify_small_values() {
    assert_eq!(sizify(1000, false), "1000");
    assert_eq!(sizify(1024, false), "1K");
  }

  #[test]
  fn test_sizify_larger_values() {
    assert_eq!(sizify(1048576, false), "1M");
    assert_eq!(sizify(1073741824, false), "1G");
    assert_eq!(sizify(1099511627776, false), "1T");
    assert_eq!(sizify(1125899906842624, false), "1P");
    assert_eq!(sizify(1152921504606846976, false), "1E");
  }

  #[test]
  fn test_sizify_add_i_suffix() {
    assert_eq!(sizify(1024, true), "1Ki");
    assert_eq!(sizify(1048576, true), "1Mi");
    assert_eq!(sizify(1073741824, true), "1Gi");
  }

  #[test]
  fn test_sizify_no_i_suffix() {
    assert_eq!(sizify(1024, false), "1K");
    assert_eq!(sizify(1048576, false), "1M");
    assert_eq!(sizify(1073741824, false), "1G");
  }

  #[test]
  fn test_sizify_decimal_points() {
    assert_eq!(sizify(1500, false), "1.47K");
    assert_eq!(sizify(1500000, false), "1.44M");
    assert_eq!(sizify(1500000000, false), "1.4G");
  }

  #[test]
  fn test_sizify_edge_cases() {
    assert_eq!(sizify(1, false), "1");
    assert_eq!(sizify(1023, false), "1023");
    assert_eq!(sizify(1025, false), "1.01K");
  }
}
