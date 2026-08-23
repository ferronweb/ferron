use std::time::Duration;

use http::{header, HeaderMap, StatusCode};

use crate::config::DEFAULT_MAX_CACHE_AGE_SECS;
use crate::lscache::LiteSpeedCacheControl;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CacheScope {
    Public,
    Private,
}

impl CacheScope {
    #[inline]
    pub fn as_str(self) -> &'static str {
        match self {
            CacheScope::Public => "public",
            CacheScope::Private => "private",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct RequestCachePolicy {
    pub allow_lookup: bool,
    pub allow_store: bool,
    /// `max-age` request directive: the stored response must be at most this
    /// old to be served without revalidation.
    pub max_age: Option<Duration>,
    /// `min-fresh` request directive: at least this much freshness must remain.
    pub min_fresh: Option<Duration>,
    /// `only-if-cached` request directive: do not contact the origin.
    pub only_if_cached: bool,
    pub reason: &'static str,
}

#[derive(Clone, Debug)]
pub struct ResponseCacheDecision {
    pub store: bool,
    pub scope: Option<CacheScope>,
    pub ttl: Option<Duration>,
    pub stale_while_revalidate: Option<Duration>,
    pub stale_if_error: Option<Duration>,
    pub must_revalidate: bool,
    /// Field names listed in a `no-cache="field-names"` directive. The response
    /// may be stored, but the stored entry must not include these fields.
    pub no_cache_field_names: Vec<String>,
    pub reason: &'static str,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct StandardCacheControl {
    public: bool,
    private: bool,
    no_cache: bool,
    no_store: bool,
    max_age: Option<Duration>,
    s_maxage: Option<Duration>,
    stale_while_revalidate: Option<Duration>,
    stale_if_error: Option<Duration>,
    must_revalidate: bool,
    proxy_revalidate: bool,
    no_cache_field_names: Vec<String>,
}

pub fn parse_request_policy(headers: &HeaderMap) -> RequestCachePolicy {
    let cache_control = headers
        .get(header::CACHE_CONTROL)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let pragma = headers
        .get(header::PRAGMA)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");

    let mut max_age = None;
    let mut min_fresh = None;
    let mut only_if_cached = false;

    for part in cache_control.split(',') {
        let directive = part.trim();
        if directive.is_empty() {
            continue;
        }
        let lower = directive.to_ascii_lowercase();
        match lower.as_str() {
            "only-if-cached" => only_if_cached = true,
            // `no-transform` is accepted and has no effect: Ferron never
            // transforms stored responses.
            "no-transform" => {}
            _ => {
                if let Some((name, value)) = lower.split_once('=') {
                    match name.trim() {
                        "max-age" => {
                            if let Ok(seconds) = value.trim().parse::<u64>() {
                                max_age = Some(Duration::from_secs(seconds));
                            }
                        }
                        "min-fresh" => {
                            if let Ok(seconds) = value.trim().parse::<u64>() {
                                min_fresh = Some(Duration::from_secs(seconds));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    if contains_token(cache_control, "no-store") {
        // RFC 9111 section 5.2.1.5: request `no-store` forbids storing the
        // request and its response, but does not forbid serving a stored
        // response.
        return RequestCachePolicy {
            allow_lookup: true,
            allow_store: false,
            max_age,
            min_fresh,
            only_if_cached,
            reason: "request-no-store",
        };
    }

    if contains_token(cache_control, "no-cache")
        || contains_token(cache_control, "max-age=0")
        || contains_token(pragma, "no-cache")
    {
        return RequestCachePolicy {
            allow_lookup: true,
            allow_store: true,
            max_age,
            min_fresh,
            only_if_cached,
            reason: "request-revalidation",
        };
    }

    RequestCachePolicy {
        allow_lookup: true,
        allow_store: true,
        max_age,
        min_fresh,
        only_if_cached,
        reason: "eligible",
    }
}

/// Whether a stored response with the given age and TTL satisfies the request's
/// `max-age` and `min-fresh` directives.
///
/// RFC 9111 sections 5.2.1.3 and 5.2.1.7: the cache must not use the stored
/// response unless its age is at most `max-age` and at least `min-fresh`
/// seconds of freshness remain.
pub fn satisfies_freshness_constraints(
    policy: &RequestCachePolicy,
    age: Duration,
    ttl: Duration,
) -> bool {
    if let Some(max_age) = policy.max_age {
        if age > max_age {
            return false;
        }
    }
    if let Some(min_fresh) = policy.min_fresh {
        let remaining = ttl.saturating_sub(age);
        if remaining < min_fresh {
            return false;
        }
    }
    true
}

pub fn evaluate_response_policy(
    status: StatusCode,
    headers: &HeaderMap,
    has_authorization: bool,
    has_set_cookie: bool,
    ls_control: Option<&LiteSpeedCacheControl>,
    litespeed_override_cache_control: bool,
) -> ResponseCacheDecision {
    let standard = parse_standard_cache_control(headers);
    let litespeed_overrides_response_policy =
        litespeed_override_cache_control && ls_control.is_some();

    if (!litespeed_overrides_response_policy && standard.no_store)
        || ls_control.is_some_and(|control| control.no_store)
    {
        return ResponseCacheDecision {
            store: false,
            scope: None,
            ttl: None,
            stale_while_revalidate: None,
            stale_if_error: None,
            must_revalidate: false,
            no_cache_field_names: Vec::new(),
            reason: "response-no-store",
        };
    }

    if (!litespeed_overrides_response_policy && standard.no_cache)
        || ls_control.is_some_and(|control| control.no_cache)
    {
        return ResponseCacheDecision {
            store: false,
            scope: None,
            ttl: None,
            stale_while_revalidate: None,
            stale_if_error: None,
            must_revalidate: false,
            no_cache_field_names: Vec::new(),
            reason: "response-no-cache",
        };
    }

    // LiteSpeed headers drive the scope only when they override the policy.
    // Without the override flag, they may still reject storage (no-store /
    // no-cache), but they must not decide scope or TTL.
    let explicit_private = (litespeed_overrides_response_policy
        && ls_control.is_some_and(|control| control.private))
        || (!litespeed_overrides_response_policy && standard.private);
    let explicit_public = (litespeed_overrides_response_policy
        && ls_control.is_some_and(|control| control.public || control.shared))
        || (!litespeed_overrides_response_policy
            && (standard.public || standard.s_maxage.is_some()));

    let scope = if explicit_private {
        CacheScope::Private
    } else if explicit_public || cacheable_by_default(status) {
        CacheScope::Public
    } else {
        return ResponseCacheDecision {
            store: false,
            scope: None,
            ttl: None,
            stale_while_revalidate: None,
            stale_if_error: None,
            must_revalidate: false,
            no_cache_field_names: Vec::new(),
            reason: "not-cacheable",
        };
    };

    // RFC 9111 section 3.5: a shared cache may store a response to an authorized
    // request when the response explicitly authorizes shared caching
    // (must-revalidate, proxy-revalidate, public, or s-maxage). When
    // LiteSpeed overrides the policy, only the LiteSpeed scope directives
    // authorize shared caching; standard directives are ignored.
    let shared_cache_authorized = if litespeed_overrides_response_policy {
        explicit_public
    } else {
        explicit_public
            || standard.must_revalidate
            || standard.proxy_revalidate
            || standard.s_maxage.is_some()
    };

    if has_authorization && scope == CacheScope::Public && !shared_cache_authorized {
        return ResponseCacheDecision {
            store: false,
            scope: None,
            ttl: None,
            stale_while_revalidate: None,
            stale_if_error: None,
            must_revalidate: false,
            no_cache_field_names: Vec::new(),
            reason: "authorization-public",
        };
    }

    if scope == CacheScope::Public && has_set_cookie {
        return ResponseCacheDecision {
            store: false,
            scope: None,
            ttl: None,
            stale_while_revalidate: None,
            stale_if_error: None,
            must_revalidate: false,
            no_cache_field_names: Vec::new(),
            reason: "public-set-cookie",
        };
    }

    let Some(ttl) = choose_ttl(
        scope,
        headers,
        &standard,
        ls_control,
        litespeed_overrides_response_policy,
    ) else {
        return ResponseCacheDecision {
            store: false,
            scope: None,
            ttl: None,
            stale_while_revalidate: None,
            stale_if_error: None,
            must_revalidate: false,
            no_cache_field_names: Vec::new(),
            reason: "zero-ttl",
        };
    };

    let must_revalidate =
        standard.must_revalidate || standard.proxy_revalidate || standard.s_maxage.is_some();

    ResponseCacheDecision {
        store: true,
        scope: Some(scope),
        ttl: Some(ttl),
        stale_while_revalidate: standard.stale_while_revalidate,
        stale_if_error: standard.stale_if_error,
        must_revalidate,
        no_cache_field_names: standard.no_cache_field_names.clone(),
        reason: "storable",
    }
}

pub(crate) fn parse_standard_cache_control(headers: &HeaderMap) -> StandardCacheControl {
    let mut parsed = StandardCacheControl::default();
    for value in headers.get_all(header::CACHE_CONTROL) {
        let Some(text) = value.to_str().ok() else {
            continue;
        };
        parse_no_cache_field_names(text, &mut parsed);
        for part in text.split(',') {
            let directive = part.trim();
            if directive.is_empty() {
                continue;
            }
            match directive.to_ascii_lowercase().as_str() {
                "public" => parsed.public = true,
                "private" => parsed.private = true,
                "no-cache" => parsed.no_cache = true,
                "no-store" => parsed.no_store = true,
                "must-revalidate" => parsed.must_revalidate = true,
                "proxy-revalidate" => parsed.proxy_revalidate = true,
                _ => {
                    if let Some((name, value)) = directive.split_once('=') {
                        match name.trim().to_ascii_lowercase().as_str() {
                            "max-age" => {
                                if let Ok(seconds) = value.trim().parse::<u64>() {
                                    parsed.max_age = Some(Duration::from_secs(seconds));
                                }
                            }
                            "s-maxage" => {
                                if let Ok(seconds) = value.trim().parse::<u64>() {
                                    parsed.s_maxage = Some(Duration::from_secs(seconds));
                                }
                            }
                            "stale-while-revalidate" => {
                                if let Ok(seconds) = value.trim().parse::<u64>() {
                                    parsed.stale_while_revalidate =
                                        Some(Duration::from_secs(seconds));
                                }
                            }
                            "stale-if-error" => {
                                if let Ok(seconds) = value.trim().parse::<u64>() {
                                    parsed.stale_if_error = Some(Duration::from_secs(seconds));
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    parsed
}

/// Extract field names from `no-cache="field-names"` directives.
///
/// RFC 9111 section 5.2.2.3: when the directive carries field names, the response
/// may still be stored, but the stored copy must not include those fields. The
/// quoted list can contain commas, so this runs before the header is split on
/// commas. `no-cache=""` is treated like bare `no-cache`.
fn parse_no_cache_field_names(text: &str, parsed: &mut StandardCacheControl) {
    let lower = text.to_ascii_lowercase();
    let mut rest = lower.as_str();
    while let Some(start) = rest.find("no-cache=\"") {
        let after = &rest[start + "no-cache=\"".len()..];
        let Some(end) = after.find('"') else {
            break;
        };
        let list = &after[..end];
        let names: Vec<String> = list
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .collect();
        if names.is_empty() {
            parsed.no_cache = true;
        } else {
            parsed.no_cache_field_names.extend(names);
        }
        rest = &after[end + 1..];
    }
}

pub(crate) fn choose_ttl(
    scope: CacheScope,
    headers: &HeaderMap,
    standard: &StandardCacheControl,
    ls_control: Option<&LiteSpeedCacheControl>,
    litespeed_overrides_response_policy: bool,
) -> Option<Duration> {
    // RFC 9111 section 4.2.1: freshness lifetime is the FIRST applicable directive,
    // not the minimum of all candidates. A shared cache uses, in order:
    // s-maxage, max-age, Expires-Date, then the heuristic.
    // `None` signals that no freshness lifetime can be derived (not storable).
    if litespeed_overrides_response_policy {
        // LiteSpeed headers fully replace standard Cache-Control.
        if scope == CacheScope::Public {
            if let Some(ttl) = ls_control.and_then(|control| control.s_maxage) {
                return Some(ttl);
            }
        }

        if let Some(ttl) = ls_control.and_then(|control| control.max_age) {
            return Some(ttl);
        }

        return Some(Duration::from_secs(DEFAULT_MAX_CACHE_AGE_SECS));
    }

    // Standard Cache-Control is the authority. LiteSpeed headers act only as
    // a fallback when they override the policy; otherwise they never drive TTL.
    if scope == CacheScope::Public {
        if let Some(ttl) = standard.s_maxage {
            return Some(ttl);
        }
    }

    if let Some(ttl) = standard.max_age {
        return Some(ttl);
    }

    if let Some(ttl) = expires_delta(headers) {
        return Some(ttl);
    }

    // RFC 9111 section 4.2.4: a cache MUST NOT generate a heuristic freshness
    // lifetime when must-revalidate (or proxy-revalidate / s-maxage) is
    // present. This also suppresses the LiteSpeed fallback, which would
    // otherwise generate a lifetime the origin did not authorize.
    if standard.must_revalidate || standard.proxy_revalidate || standard.s_maxage.is_some() {
        return None;
    }

    if litespeed_overrides_response_policy {
        if scope == CacheScope::Public {
            if let Some(ttl) = ls_control.and_then(|control| control.s_maxage) {
                return Some(ttl);
            }
        }

        if let Some(ttl) = ls_control.and_then(|control| control.max_age) {
            return Some(ttl);
        }
    }

    Some(Duration::from_secs(DEFAULT_MAX_CACHE_AGE_SECS))
}

/// Recalculate freshness parameters from updated headers during 304 revalidation.
///
/// Per RFC 9111 section 4.3.4, the stored response's freshness must be updated
/// from the 304 response's headers. This function re-parses Cache-Control (and
/// optionally LiteSpeed Cache-Control) from the merged headers and returns
/// the new TTL, stale-while-revalidate, stale-if-error, and must-revalidate.
pub(crate) fn recalculate_freshness(
    scope: CacheScope,
    headers: &HeaderMap,
    ls_control: Option<&LiteSpeedCacheControl>,
    litespeed_override_cache_control: bool,
) -> (Duration, Option<Duration>, Option<Duration>, bool) {
    let standard = parse_standard_cache_control(headers);
    let litespeed_overrides = litespeed_override_cache_control && ls_control.is_some();

    // A stored entry whose merged headers carry no explicit freshness (for
    // example must-revalidate without a lifetime) becomes perpetually stale:
    // it stays stored but must revalidate on every request.
    let ttl = choose_ttl(scope, headers, &standard, ls_control, litespeed_overrides)
        .unwrap_or(Duration::ZERO);

    let must_revalidate =
        standard.must_revalidate || standard.proxy_revalidate || standard.s_maxage.is_some();

    (
        ttl,
        standard.stale_while_revalidate,
        standard.stale_if_error,
        must_revalidate,
    )
}

/// Remove the fields listed in a `no-cache="field-names"` directive from the
/// stored entry's headers.
pub(crate) fn strip_no_cache_fields(headers: &mut HeaderMap, field_names: &[String]) {
    for field_name in field_names {
        if let Ok(name) = http::header::HeaderName::from_bytes(field_name.as_bytes()) {
            headers.remove(name);
        }
    }
}

#[inline]
fn expires_delta(headers: &HeaderMap) -> Option<Duration> {
    let expires = headers.get(header::EXPIRES)?.to_str().ok()?;
    let expires_at = httpdate::parse_http_date(expires).ok()?;
    let date = headers
        .get(header::DATE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| httpdate::parse_http_date(value).ok())
        .unwrap_or_else(std::time::SystemTime::now);

    Some(expires_at.duration_since(date).unwrap_or(Duration::ZERO))
}

#[inline]
fn cacheable_by_default(status: StatusCode) -> bool {
    matches!(
        status.as_u16(),
        200 | 203 | 204 | 206 | 300 | 301 | 308 | 404 | 405 | 410 | 414 | 501
    )
}

#[inline]
fn contains_token(value: &str, token: &str) -> bool {
    value
        .split(',')
        .map(str::trim)
        .any(|directive| directive.eq_ignore_ascii_case(token))
}

#[cfg(test)]
mod tests;
