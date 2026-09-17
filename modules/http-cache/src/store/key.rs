use ahash::AHashMap;
use http::header::{HeaderMap, HeaderName};
use rustc_hash::FxHashMap;

use crate::policy::CacheScope;

use super::types::VaryRule;

/// Maximum length of a resolved vary value admitted into a cache key.
///
/// Values are operator-controlled `set_var` outputs and must stay
/// low-entropy labels (device class, country); truncation bounds key growth
/// the same way private-key cookie values are bounded.
pub const MAX_VARY_VALUE_LEN: usize = 256;

/// Resolve an `X-LiteSpeed-Vary: value=<name>` dimension for a request.
///
/// Returns `None` when the stored variant declares no value dimension, and
/// `Some(normalized)` otherwise — empty when the variable is unset, so the
/// default variant still keys distinctly from labeled ones.
pub fn resolve_vary_value(
    vary: &VaryRule,
    variables: &FxHashMap<String, String>,
) -> Option<String> {
    let name = vary.value.as_ref()?;
    let raw = variables.get(name).map(String::as_str).unwrap_or("");
    let mut normalized = normalize_key_value(raw);
    if normalized.len() > MAX_VARY_VALUE_LEN {
        normalized.truncate(normalized.floor_char_boundary(MAX_VARY_VALUE_LEN));
    }
    Some(normalized)
}

pub fn build_entry_key(
    base_key: &str,
    scope: CacheScope,
    private_key: Option<&str>,
    vary: &VaryRule,
    headers: &HeaderMap,
    cookies: &AHashMap<String, String>,
    variables: &FxHashMap<String, String>,
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

    // Automatic vary cookies (`_lscache_vary*`): always part of the key unless
    // the stored response opted out with `no-vary`. They are appended after
    // the explicit vary cookies in sorted order so the key stays stable.
    // Names already listed explicitly are skipped here to avoid duplication.
    if !vary.no_vary {
        let mut default_names: Vec<&String> = cookies
            .keys()
            .filter(|name| {
                crate::lscache::is_default_vary_cookie_name(name)
                    && !vary.cookie_names.iter().any(|listed| listed == *name)
            })
            .collect();
        default_names.sort_unstable();
        for cookie_name in default_names {
            key.push('\n');
            key.push_str("c:");
            key.push_str(cookie_name);
            key.push('=');
            if let Some(value) = cookies.get(cookie_name) {
                key.push_str(&normalize_key_value(value));
            }
        }
    }

    if let Some(resolved) = resolve_vary_value(vary, variables) {
        key.push('\n');
        key.push_str("v:");
        key.push_str(&resolved);
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
    let mut iter = headers
        .get_all(name)
        .into_iter()
        .filter_map(|v| v.to_str().ok());
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

    use super::{build_entry_key, normalize_key_value, resolve_vary_value, VaryRule};

    fn vary_on(headers: &[HeaderName]) -> VaryRule {
        VaryRule {
            header_names: headers.to_vec(),
            cookie_names: Vec::new(),
            value: None,
            no_vary: false,
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
            &rustc_hash::FxHashMap::default(),
        );

        assert!(key.contains("h:accept-language=de de, en fr"), "{key}");
    }

    #[test]
    fn header_value_trim_and_collapse() {
        assert_eq!(normalize_key_value("  gzip\tbr  "), "gzip br");
    }

    #[test]
    fn default_vary_cookies_join_key_without_configuration() {
        let mut cookies: ahash::AHashMap<String, String> = Default::default();
        cookies.insert("_lscache_vary".to_string(), "logged-in".to_string());
        cookies.insert("tracking".to_string(), "uuid".to_string());

        let key = build_entry_key(
            "base",
            CacheScope::Public,
            None,
            &vary_on(&[]),
            &HeaderMap::new(),
            &cookies,
            &rustc_hash::FxHashMap::default(),
        );

        assert!(key.contains("c:_lscache_vary=logged-in"), "{key}");
        assert!(!key.contains("tracking"), "{key}");
    }

    #[test]
    fn default_vary_cookies_distinguish_values_and_absence() {
        let mut cookies_a: ahash::AHashMap<String, String> = Default::default();
        cookies_a.insert("_lscache_vary".to_string(), "Alabama".to_string());
        let mut cookies_b: ahash::AHashMap<String, String> = Default::default();
        cookies_b.insert("_lscache_vary".to_string(), "California".to_string());

        let key_a = build_entry_key(
            "base",
            CacheScope::Public,
            None,
            &vary_on(&[]),
            &HeaderMap::new(),
            &cookies_a,
            &rustc_hash::FxHashMap::default(),
        );
        let key_b = build_entry_key(
            "base",
            CacheScope::Public,
            None,
            &vary_on(&[]),
            &HeaderMap::new(),
            &cookies_b,
            &rustc_hash::FxHashMap::default(),
        );
        let key_none = build_entry_key(
            "base",
            CacheScope::Public,
            None,
            &vary_on(&[]),
            &HeaderMap::new(),
            &Default::default(),
            &rustc_hash::FxHashMap::default(),
        );

        assert_ne!(key_a, key_b);
        assert_ne!(key_a, key_none);
        assert_ne!(key_b, key_none);
        // Same value as request A hits the same key.
        let key_a_repeat = build_entry_key(
            "base",
            CacheScope::Public,
            None,
            &vary_on(&[]),
            &HeaderMap::new(),
            &cookies_a,
            &rustc_hash::FxHashMap::default(),
        );
        assert_eq!(key_a, key_a_repeat);
    }

    #[test]
    fn default_vary_cookies_sorted_and_not_duplicated() {
        let mut cookies: ahash::AHashMap<String, String> = Default::default();
        cookies.insert("_lscache_vary_z".to_string(), "1".to_string());
        cookies.insert("_lscache_vary_a".to_string(), "2".to_string());

        let mut rule = vary_on(&[]);
        rule.cookie_names.push("_lscache_vary_a".to_string());

        let key = build_entry_key(
            "base",
            CacheScope::Public,
            None,
            &rule,
            &HeaderMap::new(),
            &cookies,
            &rustc_hash::FxHashMap::default(),
        );

        // The explicitly listed cookie appears once, and the other default
        // cookie follows in sorted order.
        assert_eq!(key.matches("_lscache_vary_a=2").count(), 1, "{key}");
        let pos_a = key.find("_lscache_vary_a=2").unwrap();
        let pos_z = key.find("c:_lscache_vary_z=1").unwrap();
        assert!(pos_a < pos_z, "{key}");
    }

    #[test]
    fn default_vary_cookies_suppressed_by_no_vary() {
        let mut cookies: ahash::AHashMap<String, String> = Default::default();
        cookies.insert("_lscache_vary".to_string(), "logged-in".to_string());

        let mut rule = vary_on(&[]);
        rule.no_vary = true;

        let key = build_entry_key(
            "base",
            CacheScope::Public,
            None,
            &rule,
            &HeaderMap::new(),
            &cookies,
            &rustc_hash::FxHashMap::default(),
        );

        assert!(!key.contains("_lscache_vary"), "{key}");
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
            &rustc_hash::FxHashMap::default(),
        );

        assert!(key.contains("c:session=abc def"), "{key}");
    }

    fn vars(pairs: &[(&str, &str)]) -> rustc_hash::FxHashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    fn vary_value_on(name: &str) -> VaryRule {
        VaryRule {
            header_names: Vec::new(),
            cookie_names: Vec::new(),
            value: Some(name.to_string()),
            no_vary: false,
        }
    }

    #[test]
    fn vary_value_resolves_request_variable() {
        let rule = vary_value_on("device_class");
        assert_eq!(
            resolve_vary_value(&rule, &vars(&[("device_class", "mobile")])),
            Some("mobile".to_string())
        );
        // Unset variable resolves to the default (empty) variant.
        assert_eq!(resolve_vary_value(&rule, &vars(&[])), Some(String::new()));
        // No declared dimension means no key component at all.
        assert_eq!(resolve_vary_value(&vary_on(&[]), &vars(&[])), None);
    }

    #[test]
    fn vary_value_is_normalized_and_bounded() {
        let rule = vary_value_on("device_class");
        assert_eq!(
            resolve_vary_value(&rule, &vars(&[("device_class", "  a\tb  ")])),
            Some("a b".to_string())
        );
        let long = "x".repeat(super::MAX_VARY_VALUE_LEN + 100);
        let resolved = resolve_vary_value(&rule, &vars(&[("device_class", &long)])).unwrap();
        assert_eq!(resolved.len(), super::MAX_VARY_VALUE_LEN);
    }

    #[test]
    fn vary_value_partitions_entry_key() {
        let rule = vary_value_on("device_class");
        let headers = HeaderMap::new();
        let cookies: ahash::AHashMap<String, String> = Default::default();
        let mobile = build_entry_key(
            "base",
            CacheScope::Public,
            None,
            &rule,
            &headers,
            &cookies,
            &vars(&[("device_class", "mobile")]),
        );
        let desktop = build_entry_key(
            "base",
            CacheScope::Public,
            None,
            &rule,
            &headers,
            &cookies,
            &vars(&[]),
        );
        assert!(mobile.contains("\nv:mobile"), "{mobile}");
        assert!(desktop.contains("\nv:"), "{desktop}");
        assert_ne!(mobile, desktop);
        // Repeating the same variable value hits the same key.
        let mobile_repeat = build_entry_key(
            "base",
            CacheScope::Public,
            None,
            &rule,
            &headers,
            &cookies,
            &vars(&[("device_class", "mobile")]),
        );
        assert_eq!(mobile, mobile_repeat);
    }
}
