#![no_main]

use std::time::Duration;

use ferron_http_cache::lscache::{
    parse_litespeed_cache_control, parse_litespeed_purge, parse_litespeed_tags,
    parse_litespeed_vary, PurgeSelector, LS_CACHE_CONTROL, LS_PURGE, LS_TAG, LS_VARY,
};
use ferron_http_cache::policy::CacheScope;
use http::{HeaderMap, HeaderValue};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut segments = data.split(|b| *b == b'\n' || *b == b'\0');

    let ls_cc_bytes = segments.next().unwrap_or(b"");
    let vary_bytes = segments.next().unwrap_or(b"");
    let tag_bytes = segments.next().unwrap_or(b"");
    let purge_bytes = segments.next().unwrap_or(b"");

    let scope = data
        .last()
        .map(|b| {
            if *b % 2 == 0 {
                CacheScope::Public
            } else {
                CacheScope::Private
            }
        })
        .unwrap_or(CacheScope::Public);

    let mut headers = HeaderMap::new();

    maybe_insert(&mut headers, &LS_CACHE_CONTROL, ls_cc_bytes);
    maybe_insert(&mut headers, &LS_VARY, vary_bytes);
    maybe_insert(&mut headers, &LS_TAG, tag_bytes);
    maybe_insert(&mut headers, &LS_PURGE, purge_bytes);

    // Exercise all four parsers with the same header map
    let cc_control = parse_litespeed_cache_control(&headers);
    let vary = parse_litespeed_vary(&headers);
    let tags = parse_litespeed_tags(&headers, scope);
    let purge = parse_litespeed_purge(&headers);

    // --- Invariants for parse_litespeed_cache_control ---
    if let Some(ref cc) = cc_control {
        if let Some(age) = cc.max_age {
            assert!(age >= Duration::ZERO);
        }
        if let Some(age) = cc.s_maxage {
            assert!(age >= Duration::ZERO);
        }
    }

    // --- Invariants for parse_litespeed_vary ---
    let cookies = &vary.cookies;
    assert!(
        cookies.windows(2).all(|w| w[0] <= w[1]),
        "vary cookies must be sorted: {:?}",
        cookies
    );
    for name in cookies {
        assert!(!name.is_empty(), "vary cookie name must not be empty");
    }
    if let Some(ref value) = vary.value {
        assert!(!value.is_empty(), "vary value must not be empty");
    }

    // --- Invariants for parse_litespeed_tags ---
    for (i, tag) in tags.iter().enumerate() {
        for j in (i + 1)..tags.len() {
            assert!(
                !(tags[j].scope == tag.scope && tags[j].name == tag.name),
                "duplicate tag ({:?}, {})",
                tag.scope,
                tag.name
            );
        }
        assert!(!tag.name.is_empty(), "tag name must not be empty");
    }

    // --- Invariants for parse_litespeed_purge ---
    for (i, op) in purge.iter().enumerate() {
        assert!(
            !op.selectors.is_empty(),
            "purge operation {} must have at least one selector",
            i
        );
        for selector in &op.selectors {
            match selector {
                PurgeSelector::Tag(_tag) => {}
                PurgeSelector::Url(_url) => {}
                PurgeSelector::UrlPath(_path) => {}
                PurgeSelector::All => {}
            }
        }
    }
});

fn maybe_insert(headers: &mut HeaderMap, name: &http::HeaderName, bytes: &[u8]) {
    let s = match std::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => return,
    };
    if s.is_empty() {
        return;
    }
    if let Ok(v) = HeaderValue::from_str(s) {
        headers.insert(name, v);
    }
}
