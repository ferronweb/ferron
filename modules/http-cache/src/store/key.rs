use ahash::AHashMap;
use http::header::{HeaderMap, HeaderName};

use crate::policy::CacheScope;

use super::types::VaryRule;

pub fn build_entry_key(
    base_key: &str,
    scope: CacheScope,
    private_key: Option<&str>,
    vary: &VaryRule,
    headers: &HeaderMap,
    cookies: &AHashMap<String, String>,
) -> String {
    let mut key = String::with_capacity(base_key.len() + 128);
    key.push_str(base_key);
    key.push('\n');
    key.push_str("scope=");
    key.push_str(scope.as_str());

    if scope == CacheScope::Private {
        if let Some(private_key) = private_key {
            key.push('\n');
            key.push_str("private=");
            key.push_str(private_key);
        }
    }

    for name in &vary.header_names {
        key.push('\n');
        key.push_str("h:");
        key.push_str(name.as_str());
        key.push('=');
        key.push_str(&header_values(headers, name));
    }

    for cookie_name in &vary.cookie_names {
        key.push('\n');
        key.push_str("c:");
        key.push_str(cookie_name);
        key.push('=');
        if let Some(value) = cookies.get(cookie_name) {
            key.push_str(&normalize_key_value(value));
        }
    }

    if let Some(value) = &vary.value {
        key.push('\n');
        key.push_str("v:");
        key.push_str(value);
    }

    key
}

/// Normalize a vary header or cookie value for cache-key embedding: trim the
/// edges and collapse internal runs of whitespace into a single space, so
/// equivalent representations that differ only in formatting share a key.
pub fn normalize_key_value(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.is_empty() {
        return String::new();
    }
    // Fast path: already normalized -> avoid Vec allocation.
    // A normalized value has no leading/trailing whitespace, no TAB/CR/LF,
    // and no consecutive spaces.
    let mut needs_normalize = false;
    if bytes[0].is_ascii_whitespace() || bytes[bytes.len() - 1].is_ascii_whitespace() {
        needs_normalize = true;
    } else {
        let mut prev_was_space = false;
        for &b in bytes {
            if b == b' ' {
                if prev_was_space {
                    needs_normalize = true;
                    break;
                }
                prev_was_space = true;
            } else if b.is_ascii_whitespace() {
                needs_normalize = true;
                break;
            } else {
                prev_was_space = false;
            }
        }
    }
    if !needs_normalize {
        return value.to_string();
    }
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn header_values(headers: &HeaderMap, name: &HeaderName) -> String {
    let mut iter = headers.get_all(name).into_iter().filter_map(|v| v.to_str().ok());
    let Some(first) = iter.next() else {
        return String::new();
    };
    let first_norm = normalize_key_value(first);
    let Some(second) = iter.next() else {
        return first_norm;
    };
    // Two or more values: collect, sort, join.
    let mut values = Vec::with_capacity(4);
    values.push(first_norm);
    values.push(normalize_key_value(second));
    for v in iter {
        values.push(normalize_key_value(v));
    }
    values.sort_unstable();
    values.join(", ")
}

#[cfg(test)]
mod tests {
    use http::header::{HeaderName, ACCEPT_LANGUAGE};
    use http::HeaderMap;

    use crate::policy::CacheScope;

    use super::{build_entry_key, normalize_key_value, VaryRule};

    fn vary_on(headers: &[HeaderName]) -> VaryRule {
        VaryRule {
            header_names: headers.to_vec(),
            cookie_names: Vec::new(),
            value: None,
        }
    }

    #[test]
    fn header_values_collapse_whitespace_and_sort() {
        let mut headers = HeaderMap::new();
        headers.append(ACCEPT_LANGUAGE, "en  fr".parse().unwrap());
        headers.append(ACCEPT_LANGUAGE, " de\tde".parse().unwrap());

        let key = build_entry_key(
            "base",
            CacheScope::Public,
            None,
            &vary_on(&[ACCEPT_LANGUAGE]),
            &headers,
            &Default::default(),
        );

        assert!(key.contains("h:accept-language=de de, en fr"), "{key}");
    }

    #[test]
    fn header_value_trim_and_collapse() {
        assert_eq!(normalize_key_value("  gzip\tbr  "), "gzip br");
    }

    #[test]
    fn cookie_value_is_normalized_in_entry_key() {
        let mut cookies: ahash::AHashMap<String, String> = Default::default();
        cookies.insert("session".to_string(), "  abc\tdef  ".to_string());

        let mut rule = vary_on(&[]);
        rule.cookie_names.push("session".to_string());

        let key = build_entry_key(
            "base",
            CacheScope::Public,
            None,
            &rule,
            &HeaderMap::new(),
            &cookies,
        );

        assert!(key.contains("c:session=abc def"), "{key}");
    }
}
