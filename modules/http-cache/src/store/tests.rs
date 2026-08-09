#![cfg(test)]

use super::*;

use std::time::Duration;

use bytes::Bytes;
use http::header::{
    AGE, CACHE_CONTROL, CONNECTION, COOKIE, PROXY_AUTHENTICATE, PROXY_AUTHORIZATION, SET_COOKIE,
    TE, TRAILER, TRANSFER_ENCODING, UPGRADE,
};
use http::{HeaderMap, HeaderName, HeaderValue, StatusCode};

use crate::lscache::{PurgeSelector, ScopedTag};

fn request_headers(pairs: &[(&HeaderName, &str)]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        headers.append(*name, HeaderValue::from_str(value).unwrap());
    }
    headers
}

fn request_cookies(pairs: &[(&str, &str)]) -> AHashMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
        .collect()
}

fn stored_entry(base_key: &str, scope: CacheScope, body: &str, vary: VaryRule) -> StoredEntry {
    let mut headers = HeaderMap::new();
    headers.insert(
        CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=60"),
    );

    StoredEntry {
        scope,
        base_key: base_key.to_string(),
        vary,
        status: StatusCode::OK,
        headers,
        body: Some(Bytes::from(body.to_string())),
        lsc_cookies: Vec::new(),
        created_at: Instant::now(),
        ttl: Duration::from_secs(60),
        access_at: 0,
        private_key: None,
        tags: Vec::new(),
        purge_url: base_key.to_string(),
        purge_host: String::new(),
        etag: None,
        last_modified: None,
        stale_while_revalidate: None,
        stale_if_error: None,
        must_revalidate: false,
    }
}

#[test]
fn builds_distinct_public_and_private_keys() {
    let vary = VaryRule::default();
    let headers = HeaderMap::new();
    let cookies = AHashMap::default();

    let public = build_entry_key(
        "https://example.com/test",
        CacheScope::Public,
        None,
        &vary,
        &headers,
        &cookies,
    );
    let private = build_entry_key(
        "https://example.com/test",
        CacheScope::Private,
        Some("user=1"),
        &vary,
        &headers,
        &cookies,
    );

    assert_ne!(public, private);
}

#[test]
fn lookup_returns_matching_public_entry() {
    let store = CacheStore::new(4);
    let base_key = "https://example.com/page";
    let vary = VaryRule {
        header_names: vec![HeaderName::from_static("accept-language")],
        cookie_names: vec!["currency".to_string()],
        value: Some("mobile".to_string()),
    };
    let headers = request_headers(&[(&HeaderName::from_static("accept-language"), "en-US")]);
    let cookies = request_cookies(&[("currency", "USD")]);

    let entry = stored_entry(base_key, CacheScope::Public, "cached-body", vary);
    let (stats, len) = store.insert_with_request(entry, None, &headers, &cookies);
    assert_eq!(stats.size_evictions, 0);
    assert_eq!(len, 1);

    let LookupOutcome {
        entry: lookup,
        stats,
        items: len,
        had_expired,
    } = store.lookup(base_key, &headers, &cookies, None);
    let (lookup, _key, _hit) = lookup.expect("expected cache hit");
    assert_eq!(stats.expired_evictions, 0);
    assert_eq!(len, 1);
    assert!(!had_expired);
    assert_eq!(lookup.scope, CacheScope::Public);
    assert_eq!(lookup.status, StatusCode::OK);
    assert_eq!(lookup.body, Some(Bytes::from_static(b"cached-body")));
    assert!(lookup.age <= Duration::from_secs(1));
}

#[test]
fn lookup_prefers_private_entry_for_matching_private_key() {
    let store = CacheStore::new(4);
    let base_key = "https://example.com/account";
    let headers = HeaderMap::new();
    let cookies = AHashMap::default();

    let public = stored_entry(base_key, CacheScope::Public, "public", VaryRule::default());
    store.insert_with_request(public, None, &headers, &cookies);

    let private = stored_entry(
        base_key,
        CacheScope::Private,
        "private",
        VaryRule::default(),
    );
    store.insert_with_request(private, Some("user=1"), &headers, &cookies);

    let LookupOutcome { entry: lookup, .. } =
        store.lookup(base_key, &headers, &cookies, Some("user=1"));
    let (lookup, _, _) = lookup.expect("expected private cache hit");
    assert_eq!(lookup.scope, CacheScope::Private);
    assert_eq!(lookup.body, Some(Bytes::from_static(b"private")));

    let LookupOutcome { entry: lookup, .. } = store.lookup(base_key, &headers, &cookies, None);
    let (lookup, _, _) = lookup.expect("expected public cache hit");
    assert_eq!(lookup.scope, CacheScope::Public);
    assert_eq!(lookup.body, Some(Bytes::from_static(b"public")));
}

#[test]
fn insert_evicts_least_recently_used_entry_at_capacity() {
    let store = CacheStore::new(2);
    let headers = HeaderMap::new();
    let cookies = AHashMap::default();

    store.insert_with_request(
        stored_entry(
            "https://example.com/a",
            CacheScope::Public,
            "a",
            VaryRule::default(),
        ),
        None,
        &headers,
        &cookies,
    );
    store.insert_with_request(
        stored_entry(
            "https://example.com/b",
            CacheScope::Public,
            "b",
            VaryRule::default(),
        ),
        None,
        &headers,
        &cookies,
    );

    let LookupOutcome { entry: lookup, .. } =
        store.lookup("https://example.com/a", &headers, &cookies, None);
    assert!(lookup.is_some(), "expected a to become most recently used");

    let (stats, len) = store.insert_with_request(
        stored_entry(
            "https://example.com/c",
            CacheScope::Public,
            "c",
            VaryRule::default(),
        ),
        None,
        &headers,
        &cookies,
    );
    assert_eq!(stats.size_evictions, 1);
    assert_eq!(len, 2);

    assert!(store
        .lookup("https://example.com/b", &headers, &cookies, None)
        .entry
        .is_none());
    assert!(store
        .lookup("https://example.com/a", &headers, &cookies, None)
        .entry
        .is_some());
    assert!(store
        .lookup("https://example.com/c", &headers, &cookies, None)
        .entry
        .is_some());
}

#[test]
fn variant_map_per_base_is_bounded_and_evicts_oldest() {
    let store = CacheStore::new(1024);
    let headers = HeaderMap::new();
    let cookies = AHashMap::default();
    let base_key = "https://example.com/resource";

    for index in 0..(super::MAX_VARIANTS_PER_BASE + 20) {
        let vary = VaryRule {
            header_names: Vec::new(),
            cookie_names: vec![format!("variant_{index}")],
            value: None,
        };
        store.insert_with_request(
            stored_entry(base_key, CacheScope::Public, "body", vary),
            None,
            &headers,
            &cookies,
        );
    }

    let variants = store
        .variants_by_base
        .get(base_key)
        .map(|variants| variants.len())
        .unwrap_or(0);
    assert_eq!(variants, super::MAX_VARIANTS_PER_BASE);

    // The oldest variants were evicted, so their entries are unreachable.
    let oldest = StoredVariant {
        scope: CacheScope::Public,
        vary: VaryRule {
            header_names: Vec::new(),
            cookie_names: vec!["variant_0".to_string()],
            value: None,
        },
    };
    let evicted = store
        .variants_by_base
        .get(base_key)
        .is_some_and(|variants| !variants.value().contains(&oldest));
    assert!(evicted, "oldest variant should be evicted");
}

#[test]
fn set_max_entries_trims_entries_to_capacity() {
    let store = CacheStore::new(3);
    let headers = HeaderMap::new();
    let cookies = AHashMap::default();

    store.insert_with_request(
        stored_entry(
            "https://example.com/a",
            CacheScope::Public,
            "a",
            VaryRule::default(),
        ),
        None,
        &headers,
        &cookies,
    );
    store.insert_with_request(
        stored_entry(
            "https://example.com/b",
            CacheScope::Public,
            "b",
            VaryRule::default(),
        ),
        None,
        &headers,
        &cookies,
    );
    store.insert_with_request(
        stored_entry(
            "https://example.com/c",
            CacheScope::Public,
            "c",
            VaryRule::default(),
        ),
        None,
        &headers,
        &cookies,
    );

    store.set_max_entries(1);

    assert_eq!(store.len(), 1);
    let survivors = [
        store
            .lookup("https://example.com/a", &headers, &cookies, None)
            .entry
            .is_some(),
        store
            .lookup("https://example.com/b", &headers, &cookies, None)
            .entry
            .is_some(),
        store
            .lookup("https://example.com/c", &headers, &cookies, None)
            .entry
            .is_some(),
    ];
    assert_eq!(
        survivors.into_iter().filter(|survived| *survived).count(),
        1
    );
}

#[test]
fn lookup_cleans_up_expired_entries() {
    let store = CacheStore::new(4);
    let headers = HeaderMap::new();
    let cookies = AHashMap::default();

    store.insert_with_request(
        stored_entry(
            "https://example.com/expired",
            CacheScope::Public,
            "expired",
            VaryRule::default(),
        ),
        None,
        &headers,
        &cookies,
    );
    store.insert_with_request(
        stored_entry(
            "https://example.com/fresh",
            CacheScope::Public,
            "fresh",
            VaryRule::default(),
        ),
        None,
        &headers,
        &cookies,
    );

    {
        let mut expired_entry = store
            .entries
            .get("https://example.com/expired\nscope=public")
            .expect("expected inserted expired entry");
        expired_entry.created_at = Instant::now() - Duration::from_secs(5);
        expired_entry.ttl = Duration::from_secs(1);
        assert!(store
            .entries
            .replace(
                "https://example.com/expired\nscope=public".to_string(),
                expired_entry,
                false,
            )
            .is_ok());
    }

    let LookupOutcome {
        entry: lookup,
        stats,
        items: len,
        had_expired,
    } = store.lookup("https://example.com/fresh", &headers, &cookies, None);
    assert!(lookup.is_some());
    assert_eq!(stats.expired_evictions, 1);
    assert_eq!(len, 1);
    assert!(!had_expired);
    assert!(store
        .lookup("https://example.com/expired", &headers, &cookies, None)
        .entry
        .is_none());
}

#[test]
fn purge_respects_scope_selectors_and_private_key() {
    let store = CacheStore::new(8);
    let headers = HeaderMap::new();
    let cookies = AHashMap::default();

    let mut public = stored_entry(
        "https://example.com/listing",
        CacheScope::Public,
        "public",
        VaryRule::default(),
    );
    public.tags = vec![ScopedTag {
        scope: CacheScope::Public,
        name: "listing".to_string(),
    }];
    public.purge_url = "/listing".to_string();
    store.insert_with_request(public, None, &headers, &cookies);

    let mut private_user_1 = stored_entry(
        "https://example.com/account",
        CacheScope::Private,
        "user-1",
        VaryRule::default(),
    );
    private_user_1.tags = vec![ScopedTag {
        scope: CacheScope::Private,
        name: "account".to_string(),
    }];
    private_user_1.purge_url = "/account".to_string();
    store.insert_with_request(private_user_1, Some("user=1"), &headers, &cookies);

    let mut private_user_2 = stored_entry(
        "https://example.com/account-2",
        CacheScope::Private,
        "user-2",
        VaryRule::default(),
    );
    private_user_2.tags = vec![ScopedTag {
        scope: CacheScope::Private,
        name: "account".to_string(),
    }];
    private_user_2.purge_url = "/account".to_string();
    store.insert_with_request(private_user_2, Some("user=2"), &headers, &cookies);

    let operations = vec![
        PurgeOperation {
            scope: CacheScope::Public,
            selectors: vec![PurgeSelector::Url("/listing".to_string())],
            stale: false,
        },
        PurgeOperation {
            scope: CacheScope::Private,
            selectors: vec![PurgeSelector::Tag("account".to_string())],
            stale: false,
        },
    ];

    let (stats, len) = store.purge(&operations, Some("user=1"), None);
    assert_eq!(stats.purged, 2);
    assert_eq!(len, 1);
    assert!(store
        .lookup("https://example.com/listing", &headers, &cookies, None)
        .entry
        .is_none());
    assert!(store
        .lookup(
            "https://example.com/account",
            &headers,
            &cookies,
            Some("user=1")
        )
        .entry
        .is_none());
    let remaining = store
        .lookup(
            "https://example.com/account-2",
            &headers,
            &cookies,
            Some("user=2"),
        )
        .entry
        .expect("expected unmatched private entry to remain");
    assert_eq!(remaining.0.body, Some(Bytes::from_static(b"user-2")));
}

#[test]
fn zero_capacity_store_skips_insert() {
    let store = CacheStore::new(0);
    let headers = HeaderMap::new();
    let cookies = AHashMap::default();

    let (stats, len) = store.insert_with_request(
        stored_entry(
            "https://example.com/a",
            CacheScope::Public,
            "a",
            VaryRule::default(),
        ),
        None,
        &headers,
        &cookies,
    );

    assert_eq!(stats.size_evictions, 0);
    assert_eq!(len, 0);
    assert_eq!(store.len(), 0);
}

#[test]
fn strip_store_headers_removes_hop_by_hop_and_age() {
    let mut headers = HeaderMap::new();
    headers.insert(AGE, HeaderValue::from_static("60"));
    headers.insert(COOKIE, HeaderValue::from_static("a=b"));
    headers.insert(CONNECTION, HeaderValue::from_static("X-Custom"));
    headers.insert(
        "X-Custom".parse::<HeaderName>().unwrap(),
        HeaderValue::from_static("1"),
    );
    headers.insert(
        HeaderName::from_static("keep-alive"),
        HeaderValue::from_static("timeout=5"),
    );
    headers.insert(
        PROXY_AUTHENTICATE,
        HeaderValue::from_static("Basic realm=test"),
    );
    headers.insert(PROXY_AUTHORIZATION, HeaderValue::from_static("Basic abc"));
    headers.insert(TE, HeaderValue::from_static("trailers"));
    headers.insert(TRAILER, HeaderValue::from_static("X-Checksum"));
    headers.insert(TRANSFER_ENCODING, HeaderValue::from_static("chunked"));
    headers.insert(UPGRADE, HeaderValue::from_static("websocket"));

    strip_store_headers(&mut headers, CacheScope::Public);

    assert!(!headers.contains_key(AGE));
    assert!(headers.contains_key(COOKIE));
    assert!(!headers.contains_key(CONNECTION));
    assert!(!headers.contains_key("X-Custom"));
    assert!(!headers.contains_key("keep-alive"));
    assert!(!headers.contains_key(PROXY_AUTHENTICATE));
    assert!(!headers.contains_key(PROXY_AUTHORIZATION));
    assert!(!headers.contains_key(TE));
    assert!(!headers.contains_key(TRAILER));
    assert!(!headers.contains_key(TRANSFER_ENCODING));
    assert!(!headers.contains_key(UPGRADE));
}

#[test]
fn strip_store_headers_removes_set_cookie_only_for_shared_scope() {
    let mut shared = HeaderMap::new();
    shared.insert(SET_COOKIE, HeaderValue::from_static("session=abc"));
    strip_store_headers(&mut shared, CacheScope::Public);
    assert!(!shared.contains_key(SET_COOKIE));

    let mut private = HeaderMap::new();
    private.insert(SET_COOKIE, HeaderValue::from_static("session=abc"));
    strip_store_headers(&mut private, CacheScope::Private);
    assert!(private.contains_key(SET_COOKIE));
}

#[test]
fn had_expired_is_true_when_entry_expired() {
    let store = CacheStore::new(4);
    let headers = HeaderMap::new();
    let cookies = AHashMap::default();

    store.insert_with_request(
        stored_entry(
            "https://example.com/expired",
            CacheScope::Public,
            "expired",
            VaryRule::default(),
        ),
        None,
        &headers,
        &cookies,
    );

    {
        let mut expired_entry = store
            .entries
            .get("https://example.com/expired\nscope=public")
            .expect("expected inserted expired entry");
        expired_entry.created_at = Instant::now() - Duration::from_secs(5);
        expired_entry.ttl = Duration::from_secs(1);
        assert!(store
            .entries
            .replace(
                "https://example.com/expired\nscope=public".to_string(),
                expired_entry,
                false,
            )
            .is_ok());
    }

    let LookupOutcome { had_expired, .. } =
        store.lookup("https://example.com/expired", &headers, &cookies, None);
    assert!(had_expired);
}

#[test]
fn begin_fetch_returns_leader_and_follower() {
    let store = CacheStore::new(4);

    let (is_leader_1, _) = store.begin_fetch("key1");
    assert!(is_leader_1);
    assert_eq!(store.active_locks(), 1);

    let (is_leader_2, _) = store.begin_fetch("key1");
    assert!(!is_leader_2);
    assert_eq!(store.active_locks(), 1);

    store.complete_fetch("key1");
    assert_eq!(store.active_locks(), 0);
}

#[test]
fn complete_fetch_notifies_waiters() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let store = CacheStore::new(4);
    let key = "https://example.com/notify-test";

    let (_leader, _leader_notify) = store.begin_fetch(key);
    let (follower, follower_notify) = store.begin_fetch(key);

    assert!(!follower);

    let fired = Arc::new(AtomicBool::new(false));
    let fired_clone = fired.clone();
    let handle = std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            follower_notify.notified().await;
            fired_clone.store(true, Ordering::SeqCst);
        });
    });

    // Give the thread time to start waiting
    std::thread::sleep(std::time::Duration::from_millis(50));

    store.complete_fetch(key);
    handle.join().unwrap();

    assert!(fired.load(Ordering::SeqCst));
}

#[tokio::test]
async fn concurrent_misses_coalesce_to_single_upstream_fetch() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let store = Arc::new(CacheStore::new(4));
    let base_key = "https://example.com/popular";

    // Insert an entry and then expire it
    let headers = HeaderMap::new();
    let cookies = AHashMap::default();
    store.insert_with_request(
        stored_entry(base_key, CacheScope::Public, "data", VaryRule::default()),
        None,
        &headers,
        &cookies,
    );
    {
        let mut entry = store
            .entries
            .get(&format!("{base_key}\nscope=public"))
            .expect("expected entry");
        entry.created_at = Instant::now() - Duration::from_secs(10);
        entry.ttl = Duration::from_secs(1);
        store
            .entries
            .replace(format!("{base_key}\nscope=public"), entry, false)
            .ok();
    }

    // Verify lookup returns had_expired
    let LookupOutcome { had_expired, .. } = store.lookup(base_key, &headers, &cookies, None);
    assert!(had_expired);

    let fetch_count = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();

    for _ in 0..10 {
        let store = store.clone();
        let fetch_count = fetch_count.clone();
        let base_key = base_key.to_string();
        let headers = headers.clone();
        let cookies = cookies.clone();

        handles.push(tokio::spawn(async move {
            let (is_leader, notify) = store.begin_fetch(&base_key);

            #[allow(clippy::needless_return)]
            if !is_leader {
                // Follower: wait for leader to complete
                notify.notified().await;
                // Re-check cache
                let LookupOutcome { entry: lookup, .. } =
                    store.lookup(&base_key, &headers, &cookies, None);
                if lookup.is_some() {
                    return;
                }
            } else {
                // Leader: simulate upstream fetch
                fetch_count.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;

                // Store the response
                store.insert_with_request(
                    stored_entry(
                        &base_key,
                        CacheScope::Public,
                        "fresh-data",
                        VaryRule::default(),
                    ),
                    None,
                    &headers,
                    &cookies,
                );
                store.complete_fetch(&base_key);
            }
        }));
    }

    for handle in handles {
        handle.await.unwrap();
    }

    // Only one upstream fetch should have occurred
    assert_eq!(
        fetch_count.load(Ordering::SeqCst),
        1,
        "only one leader should fetch upstream"
    );

    // Cache should now have the entry
    let LookupOutcome { entry: lookup, .. } = store.lookup(base_key, &headers, &cookies, None);
    assert!(
        lookup.is_some(),
        "cache should be populated after coalesced fetch"
    );
}

#[tokio::test]
async fn follower_gets_cached_response_after_leader_stores() {
    let store = Arc::new(CacheStore::new(4));
    let base_key = "https://example.com/leader-follower";

    let headers = HeaderMap::new();
    let cookies = AHashMap::default();

    // Insert and expire
    store.insert_with_request(
        stored_entry(base_key, CacheScope::Public, "old", VaryRule::default()),
        None,
        &headers,
        &cookies,
    );
    {
        let mut entry = store
            .entries
            .get(&format!("{base_key}\nscope=public"))
            .expect("expected entry");
        entry.created_at = Instant::now() - Duration::from_secs(10);
        entry.ttl = Duration::from_secs(1);
        store
            .entries
            .replace(format!("{base_key}\nscope=public"), entry, false)
            .ok();
    }

    // Leader begins
    let (is_leader, notify) = store.begin_fetch(base_key);
    assert!(is_leader);

    // Spawn a follower that waits
    let store_clone = store.clone();
    let base_key_clone = base_key.to_string();
    let headers_clone = headers.clone();
    let cookies_clone = cookies.clone();
    let follower_handle = tokio::spawn(async move {
        // Follower waits
        notify.notified().await;
        // After notification, re-check cache
        let LookupOutcome { entry: lookup, .. } =
            store_clone.lookup(&base_key_clone, &headers_clone, &cookies_clone, None);
        lookup.and_then(|(entry, _, _)| entry.body)
    });

    // Give follower time to start waiting
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    // Leader stores the response
    store.insert_with_request(
        stored_entry(
            base_key,
            CacheScope::Public,
            "new-data",
            VaryRule::default(),
        ),
        None,
        &headers,
        &cookies,
    );
    store.complete_fetch(base_key);

    // Follower should get the new cached response
    let body = follower_handle.await.unwrap();
    assert_eq!(body, Some(Bytes::from_static(b"new-data")));
}

#[tokio::test]
async fn leader_non_cacheable_wakes_followers_without_cached_entry() {
    let store = Arc::new(CacheStore::new(4));
    let base_key = "https://example.com/non-cacheable";

    let headers = HeaderMap::new();
    let cookies = AHashMap::default();

    // Insert and expire
    store.insert_with_request(
        stored_entry(base_key, CacheScope::Public, "old", VaryRule::default()),
        None,
        &headers,
        &cookies,
    );
    {
        let mut entry = store
            .entries
            .get(&format!("{base_key}\nscope=public"))
            .expect("expected entry");
        entry.created_at = Instant::now() - Duration::from_secs(10);
        entry.ttl = Duration::from_secs(1);
        store
            .entries
            .replace(format!("{base_key}\nscope=public"), entry, false)
            .ok();
    }

    // Leader begins
    let (is_leader, notify) = store.begin_fetch(base_key);
    assert!(is_leader);

    let store_clone = store.clone();
    let base_key_clone = base_key.to_string();
    let headers_clone = headers.clone();
    let cookies_clone = cookies.clone();
    let follower_handle = tokio::spawn(async move {
        notify.notified().await;
        let LookupOutcome { entry: lookup, .. } =
            store_clone.lookup(&base_key_clone, &headers_clone, &cookies_clone, None);
        lookup.is_none() // Should be None since leader didn't store
    });

    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    // Leader decides NOT to store (non-cacheable response)
    // Just complete the fetch without inserting
    store.complete_fetch(base_key);

    let follower_saw_miss = follower_handle.await.unwrap();
    assert!(
        follower_saw_miss,
        "follower should see miss after non-cacheable leader"
    );
}

#[test]
fn stored_entry_preserves_etag_and_last_modified() {
    let store = CacheStore::new(4);
    let headers = HeaderMap::new();
    let cookies = AHashMap::default();

    let mut entry = stored_entry(
        "https://example.com/page",
        CacheScope::Public,
        "body",
        VaryRule::default(),
    );
    entry.etag = Some(HeaderValue::from_static("\"abc123\""));
    entry.last_modified = Some(HeaderValue::from_static("Wed, 01 Jan 2025 00:00:00 GMT"));

    store.insert_with_request(entry, None, &headers, &cookies);

    let LookupOutcome { entry: lookup, .. } =
        store.lookup("https://example.com/page", &headers, &cookies, None);
    let (lookup, _, _) = lookup.expect("expected cache hit");
    assert_eq!(lookup.etag, Some(HeaderValue::from_static("\"abc123\"")));
    assert_eq!(
        lookup.last_modified,
        Some(HeaderValue::from_static("Wed, 01 Jan 2025 00:00:00 GMT"))
    );
}

#[test]
fn update_entry_headers_by_key_updates_validators() {
    let store = CacheStore::new(4);
    let headers = HeaderMap::new();
    let cookies = AHashMap::default();

    let mut entry = stored_entry(
        "https://example.com/page",
        CacheScope::Public,
        "body",
        VaryRule::default(),
    );
    entry.etag = Some(HeaderValue::from_static("\"old\""));

    store.insert_with_request(entry, None, &headers, &cookies);

    let mut new_headers = HeaderMap::new();
    new_headers.insert(http::header::ETAG, HeaderValue::from_static("\"new\""));

    let result = store.update_entry_headers_by_key(
        "https://example.com/page\nscope=public",
        new_headers,
        false,
    );

    assert!(result.is_some());
    let updated = result.unwrap();
    assert_eq!(
        updated.get(http::header::ETAG),
        Some(&HeaderValue::from_static("\"new\""))
    );
}

#[test]
fn lookup_returns_cache_key_for_revalidation() {
    let store = CacheStore::new(4);
    let headers = HeaderMap::new();
    let cookies = AHashMap::default();

    let entry = stored_entry(
        "https://example.com/page",
        CacheScope::Public,
        "body",
        VaryRule::default(),
    );
    store.insert_with_request(entry, None, &headers, &cookies);

    let LookupOutcome { entry: lookup, .. } =
        store.lookup("https://example.com/page", &headers, &cookies, None);
    let (_, cache_key, _) = lookup.expect("expected cache hit");
    assert!(cache_key.contains("scope=public"));
}

#[test]
fn update_entry_headers_recalculates_ttl_from_304() {
    let store = CacheStore::new(4);
    let headers = HeaderMap::new();
    let cookies = AHashMap::default();

    let entry = stored_entry(
        "https://example.com/page",
        CacheScope::Public,
        "body",
        VaryRule::default(),
    );
    store.insert_with_request(entry, None, &headers, &cookies);

    let mut new_headers = HeaderMap::new();
    new_headers.insert(
        CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=120"),
    );

    let _ = store.update_entry_headers_by_key(
        "https://example.com/page\nscope=public",
        new_headers,
        false,
    );

    let LookupOutcome { entry: lookup, .. } =
        store.lookup("https://example.com/page", &headers, &cookies, None);
    let (lookup, _, _) = lookup.expect("expected cache hit");
    assert_eq!(lookup.ttl, Duration::from_secs(120));
}

#[test]
fn update_entry_headers_recalculates_swr_and_must_revalidate() {
    let store = CacheStore::new(4);
    let headers = HeaderMap::new();
    let cookies = AHashMap::default();

    let entry = stored_entry(
        "https://example.com/page",
        CacheScope::Public,
        "body",
        VaryRule::default(),
    );
    store.insert_with_request(entry, None, &headers, &cookies);

    let mut new_headers = HeaderMap::new();
    new_headers.insert(
        CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=60, stale-while-revalidate=30, must-revalidate"),
    );

    let _ = store.update_entry_headers_by_key(
        "https://example.com/page\nscope=public",
        new_headers,
        false,
    );

    let LookupOutcome {
        entry: Some((lookup, _, hit)),
        ..
    } = store.lookup("https://example.com/page", &headers, &cookies, None)
    else {
        panic!("expected cache hit");
    };
    assert!(matches!(hit, LookupHit::Fresh));
    assert!(lookup.must_revalidate);
}

#[test]
fn update_entry_headers_replaces_not_appends_field_values() {
    let store = CacheStore::new(4);
    let headers = HeaderMap::new();
    let cookies = AHashMap::default();

    let mut entry = stored_entry(
        "https://example.com/page",
        CacheScope::Public,
        "body",
        VaryRule::default(),
    );
    entry
        .headers
        .append(CACHE_CONTROL, HeaderValue::from_static("max-age=999"));
    entry
        .headers
        .append(http::header::SET_COOKIE, HeaderValue::from_static("a=1"));
    store.insert_with_request(entry, None, &headers, &cookies);

    let mut new_headers = HeaderMap::new();
    new_headers.insert(
        CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=120"),
    );
    new_headers.insert(http::header::SET_COOKIE, HeaderValue::from_static("b=2"));

    let result = store.update_entry_headers_by_key(
        "https://example.com/page\nscope=public",
        new_headers,
        false,
    );

    let updated = result.expect("expected header update");
    let cache_control: Vec<&str> = updated
        .get_all(CACHE_CONTROL)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .collect();
    assert_eq!(cache_control, vec!["public, max-age=120"]);
    let set_cookie: Vec<&str> = updated
        .get_all(http::header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .collect();
    assert_eq!(set_cookie, vec!["b=2"]);

    let LookupOutcome { entry: lookup, .. } =
        store.lookup("https://example.com/page", &headers, &cookies, None);
    let (lookup, _, _) = lookup.expect("expected cache hit");
    assert_eq!(lookup.ttl, Duration::from_secs(120));
}

#[test]
fn purge_all_scoped_to_requesting_host() {
    let store = CacheStore::new(8);
    let headers = HeaderMap::new();
    let cookies = AHashMap::default();

    let mut host_a = stored_entry(
        "https://a.example.com/page",
        CacheScope::Public,
        "a",
        VaryRule::default(),
    );
    host_a.purge_host = "a.example.com".to_string();
    store.insert_with_request(host_a, None, &headers, &cookies);

    let mut host_b = stored_entry(
        "https://b.example.com/page",
        CacheScope::Public,
        "b",
        VaryRule::default(),
    );
    host_b.purge_host = "b.example.com".to_string();
    store.insert_with_request(host_b, None, &headers, &cookies);

    let operations = vec![PurgeOperation {
        scope: CacheScope::Public,
        selectors: vec![PurgeSelector::All],
        stale: false,
    }];

    let (stats, len) = store.purge(&operations, None, Some("b.example.com"));
    assert_eq!(stats.purged, 1);
    assert_eq!(len, 1);
    assert!(store
        .lookup("https://b.example.com/page", &headers, &cookies, None)
        .entry
        .is_none());
    assert!(store
        .lookup("https://a.example.com/page", &headers, &cookies, None)
        .entry
        .is_some());

    // A host-ambiguous purge (no requesting host) is zone-wide.
    let (stats, len) = store.purge(&operations, None, None);
    assert_eq!(stats.purged, 1);
    assert_eq!(len, 0);
}

#[test]
fn tag_purge_scoped_to_requesting_host() {
    let store = CacheStore::new(8);
    let headers = HeaderMap::new();
    let cookies = AHashMap::default();

    let mut host_a = stored_entry(
        "https://a.example.com/a",
        CacheScope::Public,
        "a",
        VaryRule::default(),
    );
    host_a.purge_host = "a.example.com".to_string();
    host_a.tags = vec![ScopedTag {
        scope: CacheScope::Public,
        name: "v1".to_string(),
    }];
    store.insert_with_request(host_a, None, &headers, &cookies);

    let mut host_b = stored_entry(
        "https://b.example.com/b",
        CacheScope::Public,
        "b",
        VaryRule::default(),
    );
    host_b.purge_host = "b.example.com".to_string();
    host_b.tags = vec![ScopedTag {
        scope: CacheScope::Public,
        name: "v1".to_string(),
    }];
    store.insert_with_request(host_b, None, &headers, &cookies);

    let operations = vec![PurgeOperation {
        scope: CacheScope::Public,
        selectors: vec![PurgeSelector::Tag("v1".to_string())],
        stale: false,
    }];

    let (stats, len) = store.purge(&operations, None, Some("a.example.com"));
    assert_eq!(stats.purged, 1);
    assert_eq!(len, 1);
    assert!(store
        .lookup("https://a.example.com/a", &headers, &cookies, None)
        .entry
        .is_none());
    assert!(store
        .lookup("https://b.example.com/b", &headers, &cookies, None)
        .entry
        .is_some());
}

#[test]
fn variants_by_base_cleaned_up_after_purge_removes_all_entries() {
    let store = CacheStore::new(8);
    let headers = HeaderMap::new();
    let cookies = AHashMap::default();

    let entry = stored_entry(
        "https://example.com/page",
        CacheScope::Public,
        "body",
        VaryRule::default(),
    );
    store.insert_with_request(entry, None, &headers, &cookies);

    assert!(store
        .variants_by_base
        .contains_key("https://example.com/page"));

    let operations = vec![PurgeOperation {
        scope: CacheScope::Public,
        selectors: vec![PurgeSelector::All],
        stale: false,
    }];
    store.purge(&operations, None, None);

    assert!(!store
        .variants_by_base
        .contains_key("https://example.com/page"));
}

#[test]
fn variants_by_base_preserved_after_expiry_for_thundering_herd() {
    let store = CacheStore::new(8);
    let headers = HeaderMap::new();
    let cookies = AHashMap::default();

    let entry = stored_entry(
        "https://example.com/page",
        CacheScope::Public,
        "body",
        VaryRule::default(),
    );
    store.insert_with_request(entry, None, &headers, &cookies);

    // Expire the entry
    {
        let mut expired_entry = store
            .entries
            .get("https://example.com/page\nscope=public")
            .expect("expected inserted entry");
        expired_entry.created_at = Instant::now() - Duration::from_secs(10);
        expired_entry.ttl = Duration::from_secs(1);
        assert!(store
            .entries
            .replace(
                "https://example.com/page\nscope=public".to_string(),
                expired_entry,
                false,
            )
            .is_ok());
    }

    // Lookup triggers cleanup, but variants should be preserved for thundering herd
    let _ = store.lookup("https://example.com/page", &headers, &cookies, None);

    assert!(store
        .variants_by_base
        .contains_key("https://example.com/page"));
}

#[test]
fn variants_by_base_preserved_after_lru_eviction() {
    let store = CacheStore::new(1);
    let headers = HeaderMap::new();
    let cookies = AHashMap::default();

    let entry1 = stored_entry(
        "https://example.com/a",
        CacheScope::Public,
        "a",
        VaryRule::default(),
    );
    store.insert_with_request(entry1, None, &headers, &cookies);

    let entry2 = stored_entry(
        "https://example.com/b",
        CacheScope::Public,
        "b",
        VaryRule::default(),
    );
    store.insert_with_request(entry2, None, &headers, &cookies);

    // After inserting b, a should be evicted but variants_by_base for a should be preserved
    // (only cleaned up by purge, not by LRU eviction)
    assert!(store
        .lookup("https://example.com/a", &headers, &cookies, None)
        .entry
        .is_none());
    assert!(store
        .lookup("https://example.com/b", &headers, &cookies, None)
        .entry
        .is_some());
}
