//! HTTP quality-value header parsing.
//!
//! Parses headers like `Accept`, `Accept-Encoding`, and `Accept-Language`
//! that use the `value; q=weight` format. Values without an explicit `q`
//! parameter default to `1.0`. Results are sorted by descending quality.

use std::cmp::Ordering;
use std::str::FromStr;

/// A parsed header value with an optional quality weight.
#[derive(Debug, PartialEq)]
struct HeaderValue {
    /// The header value (e.g. `"text/html"`).
    value: String,
    /// The quality weight (0.0-1.0), or `None` if not specified.
    q_value: Option<f32>,
}

impl FromStr for HeaderValue {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split(';').take(2);
        let value = parts.next().ok_or(())?.trim().to_string();

        let q_value = parts.next().map(|part| {
            part.trim()
                .strip_prefix("q=")
                .unwrap_or("0")
                .parse::<f32>()
                .unwrap_or(0.0)
        });

        Ok(HeaderValue { value, q_value })
    }
}

/// Parse an HTTP quality-value header into a sorted list of values.
///
/// Values are returned in descending quality order. Values without an
/// explicit `q` parameter default to `1.0`. Values with the same quality
/// retain their original order.
///
/// # Example
///
/// ```ignore
/// let values = parse_q_value_header("text/html; q=0.8, text/plain; q=0.5");
/// assert_eq!(values, vec!["text/html", "text/plain"]);
/// ```
pub fn parse_q_value_header(header: &str) -> Vec<String> {
    let mut values: Vec<HeaderValue> = header
        .split(',')
        .filter_map(|s| HeaderValue::from_str(s.trim()).ok())
        .collect();

    let mut last_some_q_value = None;
    for value in values.iter_mut().rev() {
        if value.q_value.is_none() {
            value.q_value = Some(last_some_q_value.unwrap_or(1.0));
        } else {
            last_some_q_value = value.q_value;
        }
    }

    values.sort_by(|a, b| b.q_value.partial_cmp(&a.q_value).unwrap_or(Ordering::Equal));

    values.into_iter().map(|v| v.value).collect::<Vec<String>>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_q_value_header() {
        let header = "text/html; q=0.8, text/plain; q=0.5, text/xml; q=0.3";
        let expected = vec!["text/html", "text/plain", "text/xml"];
        assert_eq!(parse_q_value_header(header), expected);
    }

    #[test]
    fn test_parse_q_value_header_with_out_of_order_and_sparse_q_values() {
        let header = "text/html; q=0.8, application/javascript, text/javascript; q=0.4, text/plain; q=0.5, text/xml; q=0.3";
        let expected = vec![
            "text/html",
            "text/plain",
            "application/javascript",
            "text/javascript",
            "text/xml",
        ];
        assert_eq!(parse_q_value_header(header), expected);
    }
}
