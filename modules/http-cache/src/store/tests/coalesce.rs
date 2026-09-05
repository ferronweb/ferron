use super::*;
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
async fn vary_variants_have_distinct_inflight_keys() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let store = Arc::new(CacheStore::new(4));
    let base_key = "https://example.com/vary";
    let cookies = AHashMap::default();

    let ae = HeaderName::from_static("accept-encoding");
    let vary = VaryRule {
        header_names: vec![ae.clone()],
        cookie_names: Vec::new(),
        value: None,
        no_vary: false,
    };

    let gzip_headers = request_headers(&[(&ae, "gzip")]);
    let br_headers = request_headers(&[(&ae, "br")]);

    store.insert_with_request(
        stored_entry(base_key, CacheScope::Public, "gzip-body", vary.clone()),
        None,
        &gzip_headers,
        &cookies,
    );
    store.insert_with_request(
        stored_entry(base_key, CacheScope::Public, "br-body", vary.clone()),
        None,
        &br_headers,
        &cookies,
    );

    let gzip_key = store
        .primary_candidate_key(base_key, &gzip_headers, &cookies, None)
        .expect("gzip candidate key");
    let br_key = store
        .primary_candidate_key(base_key, &br_headers, &cookies, None)
        .expect("br candidate key");
    assert_ne!(
        gzip_key, br_key,
        "distinct vary variants must map to distinct entry keys"
    );

    let (is_leader_gzip, _) = store.begin_fetch(&gzip_key);
    assert!(is_leader_gzip);
    let (is_leader_br, br_notify) = store.begin_fetch(&br_key);
    assert!(
        is_leader_br,
        "variant B must get its own in-flight slot, not coalesce onto variant A"
    );

    let fired = Arc::new(AtomicBool::new(false));
    let fired_clone = fired.clone();
    let handle = std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            br_notify.notified().await;
            fired_clone.store(true, Ordering::SeqCst);
        });
    });

    std::thread::sleep(Duration::from_millis(50));
    store.complete_fetch(&gzip_key);
    assert!(
        !handle.is_finished(),
        "variant B follower must not be woken by variant A's completion"
    );
    store.complete_fetch(&br_key);
    handle.join().unwrap();

    assert!(fired.load(Ordering::SeqCst));
}

#[tokio::test]
async fn follower_wait_times_out_when_leader_never_completes() {
    let store = Arc::new(CacheStore::new(4));
    let key = "https://example.com/hung-leader";

    let (is_leader, _leader_notify) = store.begin_fetch(key);
    assert!(is_leader);
    let (is_follower, follower_notify) = store.begin_fetch(key);
    assert!(!is_follower);

    let timed_out = tokio::time::timeout(Duration::from_millis(50), follower_notify.notified())
        .await
        .is_err();
    assert!(
        timed_out,
        "a follower must stop coalescing when the leader never completes its fetch"
    );
}

/*
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
    // A fully expired entry is cleaned up on lookup; its base key is dropped
    // too, so the miss no longer reports had_expired.
    store.last_cleanup.store(0, Ordering::Relaxed);
    let LookupOutcome {
        stats, had_expired, ..
    } = store.lookup(base_key, &headers, &cookies, None);
    assert!(!had_expired);
    assert_eq!(stats.expired_evictions, 1);
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
 */

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
