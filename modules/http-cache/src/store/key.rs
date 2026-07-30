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
            key.push_str(value);
        }
    }

    if let Some(value) = &vary.value {
        key.push('\n');
        key.push_str("v:");
        key.push_str(value);
    }

    key
}

fn header_values(headers: &HeaderMap, name: &HeaderName) -> String {
    let mut values = Vec::new();
    for value in headers.get_all(name) {
        if let Ok(value) = value.to_str() {
            values.push(value.to_string());
        }
    }
    values.join(", ")
}
