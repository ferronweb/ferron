#![no_main]

use std::time::Duration;
use std::time::Instant;

use arbitrary::Unstructured;
use bytes::Bytes;
use ferron_http_cache::lscache::ScopedTag;
use ferron_http_cache::policy::CacheScope;
use ferron_http_cache::store::{build_entry_key, CacheStore, StoredEntry, VaryRule};
use http::header::{self, HeaderName, HeaderValue};
use http::{HeaderMap, StatusCode};
use libfuzzer_sys::fuzz_target;
use rustc_hash::FxHashMap;

fuzz_target!(|data: &[u8]| {
    let mut u = Unstructured::new(data);

    let max_entries = match u.arbitrary::<u8>() {
        Ok(n) => ((n as usize) % 16) + 1,
        Err(_) => 4,
    };

    let num_cycles = match u.arbitrary::<u8>() {
        Ok(n) => ((n as usize) % 20) + 1,
        Err(_) => 5,
    };

    let store = CacheStore::new(max_entries);

    for cycle in 0..num_cycles {
        let base_key = format!("https://example.com/cycle-{}", cycle);
        let scope = if cycle % 2 == 0 {
            CacheScope::Public
        } else {
            CacheScope::Private
        };

        let num_headers = (cycle % 4) as usize;
        let num_cookies = ((cycle + 1) % 4) as usize;
        let num_vary_headers = ((cycle + 2) % 3) as usize;
        let num_vary_cookies = ((cycle + 3) % 3) as usize;

        // Build vary rule from generated data
        let mut vary_header_names = Vec::new();
        for _ in 0..num_vary_headers {
            let name = extract_string(&mut u, 32);
            if let Ok(h) = HeaderName::from_bytes(name.as_bytes()) {
                vary_header_names.push(h);
            }
        }

        let mut vary_cookie_names = Vec::new();
        for _ in 0..num_vary_cookies {
            vary_cookie_names.push(extract_string(&mut u, 32));
        }

        let vary_value = if cycle % 3 == 0 {
            Some(extract_string(&mut u, 32))
        } else {
            None
        };

        let vary = VaryRule {
            header_names: vary_header_names,
            cookie_names: vary_cookie_names,
            value: vary_value,
        };

        // Build request headers
        let mut headers = HeaderMap::new();
        for _ in 0..num_headers {
            let name = extract_string(&mut u, 32);
            let value = extract_string(&mut u, 64);
            if let Ok(h) = HeaderName::from_bytes(name.as_bytes()) {
                if let Ok(v) = HeaderValue::from_str(&value) {
                    headers.insert(h, v);
                }
            }
        }

        // Build request cookies
        let mut cookies = FxHashMap::default();
        for _ in 0..num_cookies {
            let name = extract_string(&mut u, 32);
            let value = extract_string(&mut u, 64);
            cookies.insert(name, value);
        }

        let private_key: Option<String> = if scope == CacheScope::Private {
            Some(extract_string(&mut u, 64))
        } else {
            None
        };

        // --- Invariant 1: build_entry_key must not panic ---
        let key = build_entry_key(
            &base_key,
            scope,
            private_key.as_deref(),
            &vary,
            &headers,
            &cookies,
        );

        // Key must contain the base_key
        assert!(
            key.contains(&base_key) || base_key.is_empty(),
            "key must contain base_key: key={}, base_key={}",
            key,
            base_key
        );

        // Key must contain the scope
        assert!(
            key.contains(&format!("scope={}", scope.as_str())),
            "key must contain scope: key={}, scope={:?}",
            key,
            scope
        );

        // --- Invariant 2: Round-trip insert + lookup ---
        if cycle % 2 == 0 {
            let mut response_headers = HeaderMap::new();
            response_headers.insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=60"),
            );

            let entry = StoredEntry {
                scope,
                base_key: base_key.clone(),
                vary: vary.clone(),
                status: StatusCode::OK,
                headers: response_headers,
                body: Some(Bytes::from_static(b"fuzz-body")),
                lsc_cookies: Vec::new(),
                created_at: Instant::now(),
                ttl: Duration::from_secs(60),
                access_at: 0,
                private_key: private_key.clone(),
                tags: Vec::<ScopedTag>::new(),
                purge_url: base_key.clone(),
            };

            let (_stats, _len) =
                store.insert_with_request(entry, private_key.as_deref(), &headers, &cookies);

            // Lookup with the same parameters
            let (result, _stats, _len) =
                store.lookup(&base_key, &headers, &cookies, private_key.as_deref());

            assert!(
                result.is_some(),
                "round-trip: inserted entry should be findable (cycle={}, key={})",
                cycle,
                base_key
            );

            // Lookup with different headers (flipped byte values)
            let mut diff_headers = HeaderMap::new();
            for (name, value) in headers.iter() {
                let flipped = HeaderValue::from_str(
                    &String::from_utf8_lossy(value.as_bytes())
                        .chars()
                        .map(|c| (c as u8).wrapping_add(1) as char)
                        .collect::<String>(),
                )
                .unwrap_or_else(|_| HeaderValue::from_static("x"));
                diff_headers.insert(name.clone(), flipped);
            }

            let (_result2, _stats2, _len2) =
                store.lookup(&base_key, &diff_headers, &cookies, private_key.as_deref());
            // May or may not be a hit depending on vary — no invariant check here
        }
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
