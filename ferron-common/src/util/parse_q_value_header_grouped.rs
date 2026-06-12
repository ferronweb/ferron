use std::{cmp::Ordering, collections::BTreeSet, str::FromStr};

#[derive(Debug, Clone, PartialEq)]
struct HeaderValue {
  value: String,
  q_value: Option<f32>,
}

impl FromStr for HeaderValue {
  type Err = ();

  fn from_str(s: &str) -> Result<Self, Self::Err> {
    let mut parts = s.split(';').take(2);
    let value = parts.next().ok_or(())?.trim().to_string();

    let q_value = parts.next().map(|part| {
      part
        .trim()
        .strip_prefix("q=")
        .unwrap_or("0")
        .parse::<f32>()
        .unwrap_or(0.0)
    });

    Ok(HeaderValue { value, q_value })
  }
}

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
      BTreeSet::from(["application/javascript".to_string(), "text/javascript".to_string()]),
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
