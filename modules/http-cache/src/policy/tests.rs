use super::*;
use http::HeaderValue;

#[test]
fn request_no_store_allows_lookup_but_not_store() {
    let mut headers = HeaderMap::new();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    let policy = parse_request_policy(&headers);
    // RFC 9111 §5.2.1.5: no-store forbids storing, not serving.
    assert!(policy.allow_lookup);
    assert!(!policy.allow_store);
    assert_eq!(policy.reason, "request-no-store");
}

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
    let decision = evaluate_response_policy(StatusCode::OK, &headers, false, false, None, false);
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
    let decision = evaluate_response_policy(StatusCode::OK, &headers, false, false, None, false);
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
    let decision = evaluate_response_policy(StatusCode::OK, &headers, false, false, None, false);
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
    let decision = evaluate_response_policy(StatusCode::OK, &headers, false, false, None, false);
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
    let decision = evaluate_response_policy(StatusCode::OK, &headers, false, false, None, false);
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
    let decision = evaluate_response_policy(StatusCode::OK, &headers, false, false, None, false);
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
    let decision = evaluate_response_policy(StatusCode::OK, &headers, false, false, None, false);
    assert!(decision.store);
    assert_eq!(decision.ttl, Some(Duration::from_secs(60)));
}

#[test]
fn litespeed_ttl_ignored_without_override() {
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
    // Standard directives are silent, so the LS TTL must not apply; the
    // response falls back to the heuristic lifetime instead.
    assert_eq!(
        decision.ttl,
        Some(Duration::from_secs(DEFAULT_MAX_CACHE_AGE_SECS))
    );
}

#[test]
fn litespeed_headers_do_not_drive_scope_without_override() {
    // No standard directives at all: only the LS header marks the
    // response public with a long TTL.
    let headers = HeaderMap::new();
    let ls_control = LiteSpeedCacheControl {
        public: true,
        max_age: Some(Duration::from_secs(3600)),
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
    // 200 OK is still cacheable by the default heuristic, but not as a
    // 3600-second public response.
    assert!(decision.store);
    assert_eq!(
        decision.ttl,
        Some(Duration::from_secs(DEFAULT_MAX_CACHE_AGE_SECS))
    );
    assert_ne!(decision.ttl, Some(Duration::from_secs(3600)));
}

#[test]
fn litespeed_private_ignored_without_override() {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=120"),
    );
    let ls_control = LiteSpeedCacheControl {
        private: true,
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
    // Without the override, the LS `private` directive does not turn the
    // standard public response into a private one.
    assert!(decision.store);
    assert_eq!(decision.scope, Some(CacheScope::Public));
    assert_eq!(decision.ttl, Some(Duration::from_secs(120)));
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
    let decision = evaluate_response_policy(StatusCode::OK, &headers, false, false, None, false);
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
    let decision = evaluate_response_policy(StatusCode::OK, &headers, false, false, None, false);
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
    let decision = evaluate_response_policy(StatusCode::OK, &headers, false, false, None, false);
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
    let decision = evaluate_response_policy(StatusCode::OK, &headers, false, false, None, false);
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
    let decision = evaluate_response_policy(StatusCode::OK, &headers, false, false, None, false);
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
    let decision = evaluate_response_policy(StatusCode::OK, &headers, false, false, None, false);
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
    let decision = evaluate_response_policy(StatusCode::OK, &headers, false, false, None, false);
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
    let decision = evaluate_response_policy(StatusCode::OK, &headers, false, false, None, false);
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
fn request_parses_max_age_and_min_fresh() {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("max-age=30, min-fresh=10"),
    );
    let policy = parse_request_policy(&headers);
    assert!(policy.allow_lookup);
    assert_eq!(policy.max_age, Some(Duration::from_secs(30)));
    assert_eq!(policy.min_fresh, Some(Duration::from_secs(10)));
    assert!(!policy.only_if_cached);
    assert_eq!(policy.reason, "eligible");
}

#[test]
fn request_parses_only_if_cached() {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("only-if-cached"),
    );
    let policy = parse_request_policy(&headers);
    assert!(policy.allow_lookup);
    assert!(policy.only_if_cached);
}

#[test]
fn request_no_transform_is_accepted() {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-transform"),
    );
    let policy = parse_request_policy(&headers);
    assert!(policy.allow_lookup);
    assert!(policy.allow_store);
}

#[test]
fn satisfies_freshness_constraints_respects_max_age_and_min_fresh() {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("max-age=30, min-fresh=10"),
    );
    let policy = parse_request_policy(&headers);
    // age 20, ttl 60 -> remaining 40 >= 10 and age <= 30
    assert!(satisfies_freshness_constraints(
        &policy,
        Duration::from_secs(20),
        Duration::from_secs(60),
    ));
    // age 40 exceeds max-age=30
    assert!(!satisfies_freshness_constraints(
        &policy,
        Duration::from_secs(40),
        Duration::from_secs(60),
    ));
    // remaining 5 < min-fresh=10
    assert!(!satisfies_freshness_constraints(
        &policy,
        Duration::from_secs(55),
        Duration::from_secs(60),
    ));
}

#[test]
fn satisfies_freshness_constraints_unconstrained_policy() {
    let headers = HeaderMap::new();
    let policy = parse_request_policy(&headers);
    assert!(satisfies_freshness_constraints(
        &policy,
        Duration::from_secs(500),
        Duration::from_secs(60),
    ));
}

#[test]
fn max_age_zero_triggers_revalidation_reason() {
    let mut headers = HeaderMap::new();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("max-age=0"));
    let policy = parse_request_policy(&headers);
    assert_eq!(policy.reason, "request-revalidation");
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
