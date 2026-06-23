#![no_main]

use std::time::Duration;

use arbitrary::Unstructured;
use ferron_http_cache::lscache::{parse_litespeed_cache_control, LS_CACHE_CONTROL};
use ferron_http_cache::policy::{evaluate_response_policy, parse_request_policy, CacheScope};
use http::header;
use http::{HeaderMap, HeaderValue, StatusCode};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut u = Unstructured::new(data);

    let status_code = match u.arbitrary::<u16>() {
        Ok(n) => StatusCode::from_u16(n % 600).unwrap_or(StatusCode::OK),
        Err(_) => StatusCode::OK,
    };

    let cache_control = extract_string(&mut u, 256);
    let pragma = extract_string(&mut u, 64);
    let expires = extract_string(&mut u, 64);
    let date = extract_string(&mut u, 64);
    let authorization = extract_string(&mut u, 64);
    let set_cookie = extract_string(&mut u, 64);
    let ls_cache_control = extract_string(&mut u, 256);

    let has_authorization_flag = match u.arbitrary::<u8>() {
        Ok(n) => n % 2 == 0,
        Err(_) => false,
    };
    let has_set_cookie_flag = match u.arbitrary::<u8>() {
        Ok(n) => n % 2 == 0,
        Err(_) => false,
    };
    let litespeed_override = match u.arbitrary::<u8>() {
        Ok(n) => n % 2 == 0,
        Err(_) => false,
    };

    // Build request headers
    let mut request_headers = HeaderMap::new();
    if !cache_control.is_empty() {
        if let Ok(v) = HeaderValue::from_str(&cache_control) {
            request_headers.insert(header::CACHE_CONTROL, v);
        }
    }
    if !pragma.is_empty() {
        if let Ok(v) = HeaderValue::from_str(&pragma) {
            request_headers.insert(header::PRAGMA, v);
        }
    }
    if has_authorization_flag && !authorization.is_empty() {
        if let Ok(v) = HeaderValue::from_str(&authorization) {
            request_headers.insert(header::AUTHORIZATION, v);
        }
    }

    // Build response headers
    let mut response_headers = HeaderMap::new();
    if !cache_control.is_empty() {
        if let Ok(v) = HeaderValue::from_str(&cache_control) {
            response_headers.insert(header::CACHE_CONTROL, v);
        }
    }
    if !expires.is_empty() {
        if let Ok(v) = HeaderValue::from_str(&expires) {
            response_headers.insert(header::EXPIRES, v);
        }
    }
    if !date.is_empty() {
        if let Ok(v) = HeaderValue::from_str(&date) {
            response_headers.insert(header::DATE, v);
        }
    }
    if has_set_cookie_flag && !set_cookie.is_empty() {
        if let Ok(v) = HeaderValue::from_str(&set_cookie) {
            response_headers.insert(header::SET_COOKIE, v);
        }
    }
    if !ls_cache_control.is_empty() {
        if let Ok(v) = HeaderValue::from_str(&ls_cache_control) {
            response_headers.insert(LS_CACHE_CONTROL, v);
        }
    }

    // --- Test parse_request_policy ---
    let request_policy = parse_request_policy(&request_headers);
    if !request_policy.allow_lookup && !request_policy.allow_store {
        assert_eq!(request_policy.reason, "request-no-store");
    }
    if !request_policy.allow_lookup && request_policy.allow_store {
        assert_eq!(request_policy.reason, "request-revalidation");
    }

    // --- Test evaluate_response_policy ---
    let ls_control = parse_litespeed_cache_control(&response_headers);
    let decision = evaluate_response_policy(
        status_code,
        &response_headers,
        has_authorization_flag,
        has_set_cookie_flag,
        ls_control.as_ref(),
        litespeed_override,
    );

    if decision.store {
        assert!(
            decision.scope.is_some(),
            "store=true but scope=None (reason: {})",
            decision.reason
        );
        assert!(
            decision.ttl.is_some(),
            "store=true but ttl=None (reason: {})",
            decision.reason
        );
        let ttl = decision.ttl.unwrap();
        assert!(ttl >= Duration::ZERO, "ttl must be non-negative");
    } else {
        assert!(
            decision.scope.is_none(),
            "store=false but scope={:?}",
            decision.scope
        );
        assert!(
            decision.ttl.is_none(),
            "store=false but ttl={:?}",
            decision.ttl
        );
    }

    if contains_directive(&response_headers, header::CACHE_CONTROL, "no-store") {
        let ls_has_no_store = ls_control.as_ref().is_some_and(|c| c.no_store);
        if litespeed_override && ls_control.is_some() && !ls_has_no_store {
            // LSCache override may bypass standard no-store
        } else {
            assert!(!decision.store, "no-store response must not be storable");
        }
    }

    if decision.scope == Some(CacheScope::Public) && has_set_cookie_flag {
        assert!(
            !decision.store,
            "public scope with set-cookie must not store"
        );
        assert_eq!(decision.reason, "public-set-cookie");
    }

    // Validate SWR/SIE fields are consistent
    if let Some(swr) = decision.stale_while_revalidate {
        assert!(decision.store, "stale_while_revalidate set but store=false");
        assert!(decision.ttl.is_some(), "stale_while_revalidate set but ttl=None");
        assert!(swr >= Duration::ZERO, "stale_while_revalidate must be non-negative");
    }
    if let Some(sie) = decision.stale_if_error {
        assert!(decision.store, "stale_if_error set but store=false");
        assert!(decision.ttl.is_some(), "stale_if_error set but ttl=None");
        assert!(sie >= Duration::ZERO, "stale_if_error must be non-negative");
    }
    if decision.must_revalidate || decision.proxy_revalidate {
        assert!(decision.store, "must_revalidate/proxy_revalidate set but store=false");
    }
});

fn extract_string(u: &mut Unstructured, max_len: usize) -> String {
    let len = match u.arbitrary::<u8>() {
        Ok(n) => (n as usize) % max_len.min(255),
        Err(_) => 0,
    };
    let bytes = match u.bytes(len) {
        Ok(b) => b,
        Err(_) => return String::new(),
    };
    String::from_utf8_lossy(bytes).to_string()
}

fn contains_directive(headers: &HeaderMap, header_name: http::HeaderName, directive: &str) -> bool {
    for value in headers.get_all(header_name) {
        let text = match value.to_str() {
            Ok(t) => t,
            Err(_) => continue,
        };
        for part in text.split(',') {
            if part.trim().eq_ignore_ascii_case(directive) {
                return true;
            }
        }
    }
    false
}
