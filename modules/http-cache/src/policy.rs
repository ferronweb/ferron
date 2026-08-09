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
            no_cache_field_names: Vec::new(),
            reason: "not-cacheable",
        };
    };

    // RFC 9111 §3.5: a shared cache may store a response to an authorized
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
/// RFC 9111 §5.2.2.3: when the directive carries field names, the response may
/// still be stored, but the stored copy must not include those fields. The
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
    // RFC 9111 §4.2.1: freshness lifetime is the FIRST applicable directive,
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
    // a fallback when the standard directives are silent.
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

    // RFC 9111 §4.2.4: a cache MUST NOT generate a heuristic freshness
    // lifetime when must-revalidate (or proxy-revalidate / s-maxage) is
    // present. This also suppresses the LiteSpeed fallback, which would
    // otherwise generate a lifetime the origin did not authorize.
    if standard.must_revalidate || standard.proxy_revalidate || standard.s_maxage.is_some() {
        return None;
    }

    if scope == CacheScope::Public {
        if let Some(ttl) = ls_control.and_then(|control| control.s_maxage) {
            return Some(ttl);
        }
    }

    if let Some(ttl) = ls_control.and_then(|control| control.max_age) {
        return Some(ttl);
    }

    Some(Duration::from_secs(DEFAULT_MAX_CACHE_AGE_SECS))
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
mod tests {
    use super::*;
    use http::HeaderValue;

    #[test]
    fn request_no_cache_enables_revalidation() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        let policy = parse_request_policy(&headers);
        assert!(policy.allow_lookup);
        assert!(policy.allow_store);
    }

    #[test]
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
    fn max_age_zero_equals_no_cache() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("max-age=0"));
        let policy = parse_request_policy(&headers);
        assert!(policy.allow_lookup);
        assert!(policy.allow_store);
        assert_eq!(policy.reason, "request-revalidation");
    }

    #[test]
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
    fn authorization_with_must_revalidate_is_cacheable() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("must-revalidate, max-age=3600"),
        );
        let decision = evaluate_response_policy(StatusCode::OK, &headers, true, false, None, false);
        assert!(decision.store);
        assert_eq!(decision.scope, Some(CacheScope::Public));
    }

    #[test]
    fn authorization_with_proxy_revalidate_is_cacheable() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("proxy-revalidate, max-age=3600"),
        );
        let decision = evaluate_response_policy(StatusCode::OK, &headers, true, false, None, false);
        assert!(decision.store);
        assert_eq!(decision.scope, Some(CacheScope::Public));
    }

    #[test]
    fn authorization_with_s_maxage_is_cacheable() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("s-maxage=3600"),
        );
        let decision = evaluate_response_policy(StatusCode::OK, &headers, true, false, None, false);
        assert!(decision.store);
        assert_eq!(decision.scope, Some(CacheScope::Public));
    }

    #[test]
    fn authorization_rejected_when_litespeed_overrides_and_standard_must_revalidate() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("must-revalidate, max-age=3600"),
        );
        // Under override, the standard directives are ignored; the LS control
        // does not authorize shared caching, so the authorized response is rejected.
        let decision = evaluate_response_policy(
            StatusCode::OK,
            &headers,
            true,
            false,
            Some(&LiteSpeedCacheControl::default()),
            true,
        );
        assert!(!decision.store);
        assert_eq!(decision.reason, "authorization-public");
    }

    #[test]
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
    fn pragma_no_cache_triggers_revalidation() {
        let mut headers = HeaderMap::new();
        headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
        let policy = parse_request_policy(&headers);
        assert!(policy.allow_lookup);
        assert!(policy.allow_store);
        assert_eq!(policy.reason, "request-revalidation");
    }

    #[test]
    fn empty_cache_control_is_eligible() {
        let headers = HeaderMap::new();
        let policy = parse_request_policy(&headers);
        assert!(policy.allow_lookup);
        assert!(policy.allow_store);
        assert_eq!(policy.reason, "eligible");
    }

    #[test]
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
    fn s_maxage_precedence_over_max_age() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=3600, s-maxage=120"),
        );
        let decision =
            evaluate_response_policy(StatusCode::OK, &headers, false, false, None, false);
        assert!(decision.store);
        // RFC 9111 §4.2.1: s-maxage is the first applicable directive for a shared cache
        assert_eq!(decision.ttl, Some(Duration::from_secs(120)));
    }

    #[test]
    fn s_maxage_beats_smaller_max_age() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=60, s-maxage=3600"),
        );
        let decision =
            evaluate_response_policy(StatusCode::OK, &headers, false, false, None, false);
        assert!(decision.store);
        // s-maxage is first-match, not the minimum of all candidates
        assert_eq!(decision.ttl, Some(Duration::from_secs(3600)));
    }

    #[test]
    fn max_age_beats_expires() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=120"),
        );
        headers.insert(
            header::DATE,
            HeaderValue::from_static("Wed, 21 Oct 2015 07:28:00 GMT"),
        );
        headers.insert(
            header::EXPIRES,
            HeaderValue::from_static("Wed, 21 Oct 2015 07:29:00 GMT"),
        );
        let decision =
            evaluate_response_policy(StatusCode::OK, &headers, false, false, None, false);
        assert!(decision.store);
        // max-age precedes Expires even when Expires-Date would be shorter
        assert_eq!(decision.ttl, Some(Duration::from_secs(120)));
    }

    #[test]
    fn expires_used_when_no_max_age() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::DATE,
            HeaderValue::from_static("Wed, 21 Oct 2015 07:28:00 GMT"),
        );
        headers.insert(
            header::EXPIRES,
            HeaderValue::from_static("Wed, 21 Oct 2015 07:29:00 GMT"),
        );
        let decision =
            evaluate_response_policy(StatusCode::OK, &headers, false, false, None, false);
        assert!(decision.store);
        assert_eq!(decision.ttl, Some(Duration::from_secs(60)));
    }

    #[test]
    fn litespeed_fallback_after_silent_standard() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("public"));
        let ls_control = LiteSpeedCacheControl {
            public: true,
            s_maxage: Some(Duration::from_secs(600)),
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
        assert!(decision.store);
        // standard directives are silent, so the LS s-maxage fallback applies first
        assert_eq!(decision.ttl, Some(Duration::from_secs(600)));
    }

    #[test]
    fn request_no_store_in_pragma_ignored() {
        // Pragma no-store is not standard; only no-cache is defined for Pragma
        let mut headers = HeaderMap::new();
        headers.insert(header::PRAGMA, HeaderValue::from_static("no-store"));
        let policy = parse_request_policy(&headers);
        assert!(policy.allow_lookup);
        assert!(policy.allow_store);
    }

    #[test]
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

        let (ttl, _swr, _sire, _must_revalidate) =
            recalculate_freshness(CacheScope::Public, &headers, Some(&ls_control), true);
        // LiteSpeed override: LS max_age (300) takes precedence
        assert_eq!(ttl, Duration::from_secs(300));
    }

    #[test]
    fn must_revalidate_without_lifetime_is_not_storable() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, must-revalidate"),
        );
        let decision =
            evaluate_response_policy(StatusCode::OK, &headers, false, false, None, false);
        assert!(!decision.store);
        assert_eq!(decision.reason, "zero-ttl");
    }

    #[test]
    fn proxy_revalidate_without_lifetime_is_not_storable() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, proxy-revalidate"),
        );
        let decision =
            evaluate_response_policy(StatusCode::OK, &headers, false, false, None, false);
        assert!(!decision.store);
        assert_eq!(decision.reason, "zero-ttl");
    }

    #[test]
    fn must_revalidate_suppresses_litespeed_fallback() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, must-revalidate"),
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
            false,
        );
        assert!(!decision.store);
        assert_eq!(decision.reason, "zero-ttl");
    }

    #[test]
    fn must_revalidate_with_expires_uses_expires() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, must-revalidate"),
        );
        headers.insert(
            header::DATE,
            HeaderValue::from_static("Wed, 21 Oct 2015 07:28:00 GMT"),
        );
        headers.insert(
            header::EXPIRES,
            HeaderValue::from_static("Wed, 21 Oct 2015 07:29:00 GMT"),
        );
        let decision =
            evaluate_response_policy(StatusCode::OK, &headers, false, false, None, false);
        assert!(decision.store);
        // Expires is an explicit lifetime, not a heuristic, so it still applies
        assert_eq!(decision.ttl, Some(Duration::from_secs(60)));
    }

    #[test]
    fn explicit_zero_s_maxage_is_stored_with_zero_ttl() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, s-maxage=0"),
        );
        let decision =
            evaluate_response_policy(StatusCode::OK, &headers, false, false, None, false);
        assert!(decision.store);
        assert_eq!(decision.ttl, Some(Duration::ZERO));
    }

    #[test]
    fn explicit_zero_max_age_is_stored_with_zero_ttl() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=0"),
        );
        let decision =
            evaluate_response_policy(StatusCode::OK, &headers, false, false, None, false);
        assert!(decision.store);
        assert_eq!(decision.ttl, Some(Duration::ZERO));
    }

    #[test]
    fn no_cache_with_field_names_is_storable() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=120, no-cache=\"Set-Cookie, X-Origin-Data\""),
        );
        let decision =
            evaluate_response_policy(StatusCode::OK, &headers, false, false, None, false);
        assert!(decision.store);
        assert_eq!(decision.ttl, Some(Duration::from_secs(120)));
        assert_eq!(
            decision.no_cache_field_names,
            vec!["set-cookie".to_string(), "x-origin-data".to_string()]
        );
    }

    #[test]
    fn bare_no_cache_is_not_storable() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=120, no-cache"),
        );
        let decision =
            evaluate_response_policy(StatusCode::OK, &headers, false, false, None, false);
        assert!(!decision.store);
        assert_eq!(decision.reason, "response-no-cache");
    }

    #[test]
    fn empty_no_cache_field_list_is_bare_no_cache() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=120, no-cache=\"\""),
        );
        let decision =
            evaluate_response_policy(StatusCode::OK, &headers, false, false, None, false);
        assert!(!decision.store);
        assert_eq!(decision.reason, "response-no-cache");
    }

    #[test]
    fn strip_no_cache_fields_removes_named_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(header::SET_COOKIE, HeaderValue::from_static("a=1"));
        headers.insert(
            http::header::HeaderName::from_static("x-origin-data"),
            HeaderValue::from_static("secret"),
        );
        headers.insert(header::ETAG, HeaderValue::from_static("\"v1\""));
        strip_no_cache_fields(
            &mut headers,
            &["set-cookie".to_string(), "x-origin-data".to_string()],
        );
        assert!(!headers.contains_key(header::SET_COOKIE));
        assert!(!headers.contains_key("x-origin-data"));
        assert!(headers.contains_key(header::ETAG));
    }

    #[test]
    fn partial_content_is_cacheable_by_default() {
        let headers = HeaderMap::new();
        let decision = evaluate_response_policy(
            StatusCode::PARTIAL_CONTENT,
            &headers,
            false,
            false,
            None,
            false,
        );
        assert!(decision.store);
        assert_eq!(decision.scope, Some(CacheScope::Public));
    }

    #[test]
    fn recalculate_freshness_must_revalidate_without_lifetime_keeps_zero_ttl() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, must-revalidate"),
        );

        let (ttl, _swr, _sire, must_revalidate) =
            recalculate_freshness(CacheScope::Public, &headers, None, false);
        assert_eq!(ttl, Duration::ZERO);
        assert!(must_revalidate);
    }
}
