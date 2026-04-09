use std::time::Duration;

use http::header;
use http::{HeaderMap, StatusCode};

use crate::config::DEFAULT_MAX_CACHE_AGE_SECS;
use crate::lscache::LiteSpeedCacheControl;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CacheScope {
    Public,
    Private,
}

impl CacheScope {
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
    pub reason: &'static str,
}

#[derive(Clone, Debug, Default)]
struct StandardCacheControl {
    public: bool,
    private: bool,
    no_cache: bool,
    no_store: bool,
    max_age: Option<Duration>,
    s_maxage: Option<Duration>,
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
            allow_lookup: false,
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
            reason: "not-cacheable",
        };
    };

    if has_authorization && scope == CacheScope::Public && !explicit_public {
        return ResponseCacheDecision {
            store: false,
            scope: None,
            ttl: None,
            reason: "authorization-public",
        };
    }

    if scope == CacheScope::Public && has_set_cookie {
        return ResponseCacheDecision {
            store: false,
            scope: None,
            ttl: None,
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
            reason: "zero-ttl",
        };
    }

    ResponseCacheDecision {
        store: true,
        scope: Some(scope),
        ttl: Some(ttl),
        reason: "storable",
    }
}

fn parse_standard_cache_control(headers: &HeaderMap) -> StandardCacheControl {
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
            match directive {
                "public" => parsed.public = true,
                "private" => parsed.private = true,
                "no-cache" => parsed.no_cache = true,
                "no-store" => parsed.no_store = true,
                _ => {
                    if let Some((name, value)) = directive.split_once('=') {
                        match name.trim() {
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
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    parsed
}

fn choose_ttl(
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

fn expires_delta(headers: &HeaderMap) -> Option<Duration> {
    let expires = headers.get(header::EXPIRES)?.to_str().ok()?;
    let expires_at = httpdate::parse_http_date(expires).ok()?;
    let date = headers
        .get(header::DATE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| httpdate::parse_http_date(value).ok())
        .unwrap_or_else(std::time::SystemTime::now);

    expires_at.duration_since(date).ok()
}

fn cacheable_by_default(status: StatusCode) -> bool {
    matches!(
        status.as_u16(),
        200 | 203 | 204 | 300 | 301 | 302 | 307 | 308 | 404 | 405 | 410 | 414 | 501
    )
}

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
    fn request_no_cache_skips_lookup() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        let policy = parse_request_policy(&headers);
        assert!(!policy.allow_lookup);
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
    fn litespeed_override_ignores_standard_no_store() {
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
            true,
        );
        assert!(decision.store);
        assert_eq!(decision.scope, Some(CacheScope::Public));
        assert_eq!(decision.ttl, Some(Duration::from_secs(120)));
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
}
