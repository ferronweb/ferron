//! HTTP quality-value header parsing with q-value grouping.
//!
//! Like [`parse_q_value_header`](crate::util::parse_q_value_header::parse_q_value_header),
//! but groups values that share the same quality weight into sets. This is
//! useful for content negotiation where multiple values at the same quality
//! are equivalent and can be tried in any order.

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::str::FromStr;

/// A parsed header value with an optional quality weight.
#[derive(Debug, Clone, PartialEq)]
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

/// Parse an HTTP quality-value header into groups of values sorted by quality.
///
/// Values with the same quality weight are grouped into a single [`BTreeSet`].
/// Groups are returned in descending quality order.
///
/// # Example
///
/// ```ignore
/// let groups = parse_q_value_header_grouped(
///     "text/html; q=0.8, text/plain; q=0.8, text/xml; q=0.5"
/// );
/// // groups[0] = {"text/html", "text/plain"} (q=0.8)
/// // groups[1] = {"text/xml"} (q=0.5)
/// ```
pub fn parse_q_value_header_grouped(header: &str) -> Vec<BTreeSet<String>> {
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

    let mut grouped: Vec<BTreeSet<String>> = Vec::new();
    if let Some(first) = values.first() {
        grouped.push(BTreeSet::from([first.value.clone()]));
    }
    for (previous, current) in values.windows(2).map(|w| (&w[0], &w[1])) {
        if current.q_value == previous.q_value {
            if let Some(last) = grouped.last_mut() {
                last.insert(current.value.clone());
            } else {
                // The grouped vector is empty...
                grouped.push(BTreeSet::from([current.value.clone()]));
            }
        } else {
            grouped.push(BTreeSet::from([current.value.clone()]));
        }
    }

    grouped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_q_value_header() {
        let header = "text/html; q=0.8, text/plain; q=0.5, text/xml; q=0.3";
        let expected = vec![
            BTreeSet::from(["text/html".to_string()]),
            BTreeSet::from(["text/plain".to_string()]),
            BTreeSet::from(["text/xml".to_string()]),
        ];
        assert_eq!(parse_q_value_header_grouped(header), expected);
    }

    #[test]
    fn test_parse_q_value_header_with_out_of_order_and_sparse_q_values() {
        let header = "text/html; q=0.8, application/javascript, text/javascript; q=0.4, text/plain; q=0.5, text/xml; q=0.3";
        let expected = vec![
            BTreeSet::from(["text/html".to_string()]),
            BTreeSet::from(["text/plain".to_string()]),
            BTreeSet::from([
                "application/javascript".to_string(),
                "text/javascript".to_string(),
            ]),
            BTreeSet::from(["text/xml".to_string()]),
        ];
        assert_eq!(parse_q_value_header_grouped(header), expected);
    }

    #[test]
    fn test_parse_q_value_header_single() {
        let header = "text/html";
        let expected = vec![BTreeSet::from(["text/html".to_string()])];
        assert_eq!(parse_q_value_header_grouped(header), expected);
    }
}
