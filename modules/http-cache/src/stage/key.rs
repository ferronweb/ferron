use ahash::{AHashMap, AHashSet};
use ferron_core::pipeline::PipelineError;
use http::header::{HeaderMap, HeaderName};

use crate::config::CacheConfig;
use crate::store::VaryRule;

pub(super) const PRIVATE_COOKIE_NAMES: &[&str] =
    &["frontend", "phpsessid", "xf_session", "lsc_private"];

/// Maximum number of cookie components admitted into a private cache key.
const MAX_PRIVATE_KEY_COOKIE_COMPONENTS: usize = 8;

/// Maximum length of a cookie value admitted into a private cache key.
const MAX_PRIVATE_KEY_COOKIE_VALUE_LEN: usize = 256;

pub(super) fn build_base_key(
    encrypted: bool,
    headers: &HeaderMap,
    original_uri: Option<&http::Uri>,
    fallback_uri: &http::Uri,
    resolved_host: Option<&str>,
) -> String {
    let uri = original_uri.unwrap_or(fallback_uri);
    let scheme = if encrypted { "https" } else { "http" };
    // Prefer the resolved vhost: the client-supplied Host header can be
    // spoofed or differ in case, which would otherwise fragment the cache
    // and let a client miss other tenants' entries.
    let host = resolved_host
        .or_else(|| {
            headers
                .get(http::header::HOST)
                .and_then(|value| value.to_str().ok())
        })
        .unwrap_or("")
        .to_ascii_lowercase();
    let path_and_query = uri
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or("/");
    let mut key = String::with_capacity(scheme.len() + host.len() + path_and_query.len() + 3);
    key.push_str(scheme);
    key.push_str("://");
    key.push_str(&host);
    key.push_str(path_and_query);
    key
}

pub(super) fn parse_cookies(headers: &HeaderMap) -> AHashMap<String, String> {
    let mut cookies = AHashMap::default();
    for value in headers.get_all(http::header::COOKIE) {
        let Some(text) = value.to_str().ok() else {
            continue;
        };
        for cookie in text.split(';') {
            let Some((name, value)) = cookie.split_once('=') else {
                continue;
            };
            let name = name.trim();
            let value = value.trim();
            if !name.is_empty() {
                cookies.insert(name.to_string(), value.to_string());
            }
        }
    }
    cookies
}

/// Build the private-scope key component for a request, or `None` when the
/// request carries no client identity.
///
/// The key requires at least one identifying component: an authenticated user,
/// a recognized private cookie, or a cookie explicitly declared as a `Vary`
/// cookie. When none is present the caller must not store in private scope
/// (`private-no-identity`); the key must never be derived from the client IP
/// alone, otherwise a CGNAT-shared address would leak one user's private
/// response to another.
pub(super) fn build_private_cache_key(
    cookies: &AHashMap<String, String>,
    auth_user: Option<&str>,
    vary_cookie_names: &[String],
) -> Option<String> {
    let mut components = Vec::new();
    if let Some(auth_user) = auth_user {
        components.push(format!("auth={auth_user}"));
    }

    let mut matched_private_cookie = false;
    for (name, value) in cookies {
        if is_private_cookie_name(name) && value.len() >= 16 {
            matched_private_cookie = true;
            components.push(format!("cookie:{name}={}", truncate_cookie_value(value)));
        }
    }

    // Without an authenticated user or a recognized private cookie, an explicit
    // Vary cookie still identifies the client. Do not fall back to every cookie:
    // arbitrary cookies would explode the private-key space.
    if !matched_private_cookie {
        for (name, value) in cookies {
            if vary_cookie_names
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(name))
            {
                components.push(format!("cookie:{name}={}", truncate_cookie_value(value)));
            }
        }
    }

    if components.is_empty() {
        return None;
    }

    components.sort_unstable();
    components.truncate(MAX_PRIVATE_KEY_COOKIE_COMPONENTS);
    Some(components.join("\0"))
}

fn truncate_cookie_value(value: &str) -> &str {
    if value.len() <= MAX_PRIVATE_KEY_COOKIE_VALUE_LEN {
        value
    } else {
        &value[..value.floor_char_boundary(MAX_PRIVATE_KEY_COOKIE_VALUE_LEN)]
    }
}

/// Request headers that select validators or byte ranges at serve time rather
/// than content-negotiated representations. Origins may list them in `Vary`
/// (the static handler does), but a cache must not fragment stored variants on
/// them: a fresh conditional request must still hit the stored representation
/// so the cache can evaluate the conditionals locally (RFC 9111 §4.1).
const NON_VARY_CONDITIONAL_HEADERS: &[HeaderName] = &[
    http::header::IF_MATCH,
    http::header::IF_MODIFIED_SINCE,
    http::header::IF_NONE_MATCH,
    http::header::IF_RANGE,
    http::header::IF_UNMODIFIED_SINCE,
    http::header::RANGE,
];

pub(super) fn build_vary_rule(
    headers: &HeaderMap,
    config: &CacheConfig,
    ls_vary: &crate::lscache::LiteSpeedVary,
) -> Result<Option<VaryRule>, PipelineError> {
    let mut header_names: AHashSet<HeaderName> = config.vary_headers.iter().cloned().collect();
    for value in headers.get_all(http::header::VARY) {
        let Some(text) = value.to_str().ok() else {
            continue;
        };
        for token in text.split(',') {
            let token = token.trim();
            if token == "*" {
                return Ok(None);
            }
            if token.is_empty() {
                continue;
            }
            let name = HeaderName::from_bytes(token.as_bytes())
                .map_err(|error| PipelineError::custom(error.to_string()))?;
            if !NON_VARY_CONDITIONAL_HEADERS.contains(&name) {
                header_names.insert(name);
            }
        }
    }
    let mut header_names: Vec<_> = header_names.into_iter().collect();
    header_names.sort_by(|left, right| left.as_str().cmp(right.as_str()));

    let mut cookie_names: AHashSet<String> = ls_vary.cookies.iter().cloned().collect();
    for name in &config.vary_cookies {
        cookie_names.insert(name.clone());
    }
    let mut cookie_names: Vec<_> = cookie_names.into_iter().collect();
    cookie_names.sort_unstable();

    Ok(Some(VaryRule {
        header_names,
        cookie_names,
        value: None,
    }))
}

#[inline]
pub(super) fn is_private_cookie_name(name: &str) -> bool {
    PRIVATE_COOKIE_NAMES
        .iter()
        .any(|candidate| name.eq_ignore_ascii_case(candidate))
        || starts_with_ignore_ascii_case(name, "wp_woocommerce_session_")
}

#[inline]
fn starts_with_ignore_ascii_case(value: &str, prefix: &str) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

/// Return a truncated representation of a cache key for use in access logs.
///
/// The query string is dropped before truncating: URLs like
/// `/search?token=<secret>` would otherwise leak up to the truncation limit
/// into the logs. The scope and `Vary` components after the base URL are
/// kept, so the fingerprint still distinguishes variants.
pub(super) fn cache_key_fingerprint(key: &str) -> String {
    const MAX_LEN: usize = 48;
    let base_end = key.find('\n').unwrap_or(key.len());
    let query_start = key[..base_end].find('?');
    let cleaned = match query_start {
        Some(query_start) => {
            let mut cleaned = String::with_capacity(key.len());
            cleaned.push_str(&key[..query_start]);
            cleaned.push_str(&key[base_end..]);
            cleaned
        }
        None => key.to_string(),
    };
    if cleaned.len() <= MAX_LEN {
        cleaned
    } else {
        format!("{}...", &cleaned[..MAX_LEN])
    }
}
