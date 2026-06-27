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

    if contains_token(cache_control, "no-store") {
        return RequestCachePolicy {
            allow_lookup: false,
            allow_store: false,
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
            reason: "request-revalidation",
        };
    }

    RequestCachePolicy {
        allow_lookup: true,
        allow_store: true,
        reason: "eligible",
    }
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
            reason: "response-no-cache",
        };
    }

    let explicit_private = ls_control.is_some_and(|control| control.private)
        || (!litespeed_overrides_response_policy && standard.private);
    let explicit_public = ls_control.is_some_and(|control| control.public || control.shared)
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
            reason: "not-cacheable",
        };
    };

    if has_authorization && scope == CacheScope::Public && !explicit_public {
        return ResponseCacheDecision {
            store: false,
            scope: None,
            ttl: None,
            stale_while_revalidate: None,
            stale_if_error: None,
            must_revalidate: false,
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
            reason: "public-set-cookie",
        };
    }

    let ttl = choose_ttl(
        scope,
        headers,
        &standard,
        ls_control,
        litespeed_overrides_response_policy,
    );
    if ttl.is_zero() {
        return ResponseCacheDecision {
            store: false,
            scope: None,
            ttl: None,
            stale_while_revalidate: None,
            stale_if_error: None,
            must_revalidate: false,
            reason: "zero-ttl",
        };
    }

    let must_revalidate =
        standard.must_revalidate || standard.proxy_revalidate || standard.s_maxage.is_some();

    ResponseCacheDecision {
        store: true,
        scope: Some(scope),
        ttl: Some(ttl),
        stale_while_revalidate: standard.stale_while_revalidate,
        stale_if_error: standard.stale_if_error,
        must_revalidate,
        reason: "storable",
    }
}

pub(crate) fn parse_standard_cache_control(headers: &HeaderMap) -> StandardCacheControl {
    let mut parsed = StandardCacheControl::default();
    for value in headers.get_all(header::CACHE_CONTROL) {
        let Some(text) = value.to_str().ok() else {
            continue;
        };
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

pub(crate) fn choose_ttl(
    scope: CacheScope,
    headers: &HeaderMap,
    standard: &StandardCacheControl,
    ls_control: Option<&LiteSpeedCacheControl>,
    litespeed_overrides_response_policy: bool,
) -> Duration {
    if litespeed_overrides_response_policy {
        if scope == CacheScope::Public {
            if let Some(ttl) = ls_control.and_then(|control| control.s_maxage) {
                return ttl;
            }
        }

        if let Some(ttl) = ls_control.and_then(|control| control.max_age) {
            return ttl;
        }

        return Duration::from_secs(DEFAULT_MAX_CACHE_AGE_SECS);
    }

    let mut ttl_candidates = Vec::new();

    if scope == CacheScope::Public {
        if let Some(ttl) = standard.s_maxage {
            ttl_candidates.push(ttl);
        }
        if let Some(ttl) = ls_control.and_then(|control| control.s_maxage) {
            ttl_candidates.push(ttl);
        }
    }

    if let Some(ttl) = standard.max_age {
        ttl_candidates.push(ttl);
    }
    if let Some(ttl) = ls_control.and_then(|control| control.max_age) {
        ttl_candidates.push(ttl);
    }
    if let Some(ttl) = expires_delta(headers) {
        ttl_candidates.push(ttl);
    }

    ttl_candidates
        .into_iter()
        .min()
        .unwrap_or_else(|| Duration::from_secs(DEFAULT_MAX_CACHE_AGE_SECS))
}

/// Recalculate freshness parameters from updated headers during 304 revalidation.
///
/// Per RFC 9111 §4.3.4, the stored response's freshness must be updated from
/// the 304 response's headers. This function re-parses Cache-Control (and
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

    let ttl = choose_ttl(
        scope,
        headers,
        &standard,
        ls_control,
        litespeed_overrides,
    );

    let must_revalidate =
        standard.must_revalidate || standard.proxy_revalidate || standard.s_maxage.is_some();

    (
        ttl,
        standard.stale_while_revalidate,
        standard.stale_if_error,
        must_revalidate,
    )
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
        200 | 203 | 204 | 300 | 301 | 308 | 404 | 405 | 410 | 414 | 501
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
mod tests {
    use super::*;
    use http::HeaderValue;

    #[test]
    #[inline]
    fn request_no_cache_enables_revalidation() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        let policy = parse_request_policy(&headers);
        assert!(policy.allow_lookup);
        assert!(policy.allow_store);
    }

    #[test]
    #[inline]
    fn response_public_is_cacheable() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=120"),
        );
        let decision =
            evaluate_response_policy(StatusCode::OK, &headers, false, false, None, false);
        assert!(decision.store);
        assert_eq!(decision.scope, Some(CacheScope::Public));
        assert_eq!(decision.ttl, Some(Duration::from_secs(120)));
    }

    #[test]
    #[inline]
    fn public_set_cookie_is_rejected() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=120"),
        );
        let decision = evaluate_response_policy(StatusCode::OK, &headers, false, true, None, false);
        assert!(!decision.store);
    }

    #[test]
    #[inline]
    fn standard_no_store_wins_without_litespeed_override() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        let ls_control = LiteSpeedCacheControl {
            public: true,
            max_age: Some(Duration::from_secs(120)),
            ..LiteSpeedCacheControl::default()
        };

        let decision = evaluate_response_policy(
            StatusCode::OK,
            &headers,
            false,
            false,
            Some(&ls_control),
            false,
        );
        assert!(!decision.store);
        assert_eq!(decision.reason, "response-no-store");
    }

    #[test]
    #[inline]
    fn litespeed_override_prefers_litespeed_ttl() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=5"),
        );
        let ls_control = LiteSpeedCacheControl {
            public: true,
            max_age: Some(Duration::from_secs(120)),
            ..LiteSpeedCacheControl::default()
        };

        let decision = evaluate_response_policy(
            StatusCode::OK,
            &headers,
            false,
            false,
            Some(&ls_control),
            true,
        );
        assert!(decision.store);
        assert_eq!(decision.ttl, Some(Duration::from_secs(120)));
    }

    #[test]
    #[inline]
    fn no_store_wins_over_no_cache() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store, no-cache"),
        );
        let decision =
            evaluate_response_policy(StatusCode::OK, &headers, false, false, None, false);
        assert!(!decision.store);
        assert_eq!(decision.reason, "response-no-store");
    }

    #[test]
    #[inline]
    fn max_age_zero_equals_no_cache() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("max-age=0"));
        let policy = parse_request_policy(&headers);
        assert!(policy.allow_lookup);
        assert!(policy.allow_store);
        assert_eq!(policy.reason, "request-revalidation");
    }

    #[test]
    #[inline]
    fn authorization_without_explicit_public_is_not_cacheable() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("max-age=3600"),
        );
        let decision = evaluate_response_policy(StatusCode::OK, &headers, true, false, None, false);
        assert!(!decision.store);
        assert_eq!(decision.reason, "authorization-public");
    }

    #[test]
    #[inline]
    fn authorization_with_explicit_public_is_cacheable() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=3600"),
        );
        let decision = evaluate_response_policy(StatusCode::OK, &headers, true, false, None, false);
        assert!(decision.store);
    }

    #[test]
    #[inline]
    fn private_set_cookie_is_allowed() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("private, max-age=3600"),
        );
        let decision = evaluate_response_policy(StatusCode::OK, &headers, false, true, None, false);
        assert!(decision.store);
        assert_eq!(decision.scope, Some(CacheScope::Private));
    }

    #[test]
    #[inline]
    fn not_cacheable_status_code() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("max-age=3600"),
        );
        // 201 Created is not cacheable by default and has no explicit public/private
        let decision =
            evaluate_response_policy(StatusCode::CREATED, &headers, false, false, None, false);
        assert!(!decision.store);
        assert_eq!(decision.reason, "not-cacheable");
    }

    #[test]
    #[inline]
    fn cacheable_by_default_status_without_explicit_directive() {
        let headers = HeaderMap::new();
        // 200 OK is cacheable by default even without explicit Cache-Control
        let decision =
            evaluate_response_policy(StatusCode::OK, &headers, false, false, None, false);
        assert!(decision.store);
        assert_eq!(decision.scope, Some(CacheScope::Public));
        assert_eq!(decision.ttl, Some(Duration::from_secs(300)));
    }

    #[test]
    #[inline]
    fn pragma_no_cache_triggers_revalidation() {
        let mut headers = HeaderMap::new();
        headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
        let policy = parse_request_policy(&headers);
        assert!(policy.allow_lookup);
        assert!(policy.allow_store);
        assert_eq!(policy.reason, "request-revalidation");
    }

    #[test]
    #[inline]
    fn empty_cache_control_is_eligible() {
        let headers = HeaderMap::new();
        let policy = parse_request_policy(&headers);
        assert!(policy.allow_lookup);
        assert!(policy.allow_store);
        assert_eq!(policy.reason, "eligible");
    }

    #[test]
    #[inline]
    fn litespeed_override_bypasses_standard_no_cache() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        let ls_control = LiteSpeedCacheControl {
            public: true,
            max_age: Some(Duration::from_secs(120)),
            ..LiteSpeedCacheControl::default()
        };
        let decision = evaluate_response_policy(
            StatusCode::OK,
            &headers,
            false,
            false,
            Some(&ls_control),
            true,
        );
        assert!(decision.store);
    }

    #[test]
    #[inline]
    fn s_maxage_precedence_over_max_age() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=3600, s-maxage=120"),
        );
        let decision =
            evaluate_response_policy(StatusCode::OK, &headers, false, false, None, false);
        assert!(decision.store);
        // s-maxage (120) should be the minimum among candidates
        assert_eq!(decision.ttl, Some(Duration::from_secs(120)));
    }

    #[test]
    #[inline]
    fn request_no_store_in_pragma_ignored() {
        // Pragma no-store is not standard; only no-cache is defined for Pragma
        let mut headers = HeaderMap::new();
        headers.insert(header::PRAGMA, HeaderValue::from_static("no-store"));
        let policy = parse_request_policy(&headers);
        assert!(policy.allow_lookup);
        assert!(policy.allow_store);
    }

    #[test]
    #[inline]
    fn recalculate_freshness_updates_ttl_and_directives() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=120, stale-while-revalidate=30"),
        );

        let (ttl, swr, sire, must_revalidate) =
            recalculate_freshness(CacheScope::Public, &headers, None, false);
        assert_eq!(ttl, Duration::from_secs(120));
        assert_eq!(swr, Some(Duration::from_secs(30)));
        assert!(!must_revalidate);
        // stale_if_error is not set in this header
        assert!(sire.is_none());
    }

    #[test]
    #[inline]
    fn recalculate_freshness_with_s_maxage_sets_must_revalidate() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, s-maxage=60"),
        );

        let (ttl, _swr, _sire, must_revalidate) =
            recalculate_freshness(CacheScope::Public, &headers, None, false);
        assert_eq!(ttl, Duration::from_secs(60));
        assert!(must_revalidate); // s-maxage implies must-revalidate
    }

    #[test]
    #[inline]
    fn recalculate_freshness_litespeed_override() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=5"),
        );
        let ls_control = LiteSpeedCacheControl {
            public: true,
            max_age: Some(Duration::from_secs(300)),
            ..LiteSpeedCacheControl::default()
        };

        let (ttl, _swr, _sire, _must_revalidate) = recalculate_freshness(
            CacheScope::Public,
            &headers,
            Some(&ls_control),
            true,
        );
        // LiteSpeed override: LS max_age (300) takes precedence
        assert_eq!(ttl, Duration::from_secs(300));
    }
}
