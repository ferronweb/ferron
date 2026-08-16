use super::*;
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
    // A 304 `Set-Cookie` must not merge into the stored entry.
    assert!(!updated.contains_key(http::header::SET_COOKIE));

    let LookupOutcome { entry: lookup, .. } =
        store.lookup("https://example.com/page", &headers, &cookies, None);
    let (lookup, _, _) = lookup.expect("expected cache hit");
    assert_eq!(lookup.ttl, Duration::from_secs(120));
    assert!(!lookup.headers.contains_key(http::header::SET_COOKIE));
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
fn variants_by_base_removed_after_expiry_cleanup() {
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

    // Expire the entry beyond its TTL and SWR window
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

    // Lookup triggers cleanup, which must drop the orphaned base key
    store
        .last_cleanup
        .store(0, std::sync::atomic::Ordering::Relaxed);
    let outcome = store.lookup("https://example.com/page", &headers, &cookies, None);
    assert_eq!(outcome.stats.expired_evictions, 1);
    assert!(outcome.entry.is_none());
    assert!(!store
        .variants_by_base
        .contains_key("https://example.com/page"));
}

#[test]
fn variants_by_base_removed_after_size_eviction() {
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
    let (stats, _) = store.insert_with_request(entry2, None, &headers, &cookies);

    // Inserting b evicts a; a's orphaned base key must be dropped
    assert_eq!(stats.size_evictions, 1);
    assert!(!store.variants_by_base.contains_key("https://example.com/a"));
    assert!(store.variants_by_base.contains_key("https://example.com/b"));
}

#[test]
fn variants_by_base_kept_when_other_variant_survives_eviction() {
    let store = CacheStore::new(1);
    let vary = VaryRule {
        header_names: vec![HeaderName::from_static("accept-language")],
        cookie_names: Vec::new(),
        value: None,
    };
    let en_headers = request_headers(&[(&HeaderName::from_static("accept-language"), "en-US")]);
    let fr_headers = request_headers(&[(&HeaderName::from_static("accept-language"), "fr-FR")]);
    let cookies = AHashMap::default();

    store.insert_with_request(
        stored_entry(
            "https://example.com/page",
            CacheScope::Public,
            "en",
            vary.clone(),
        ),
        None,
        &en_headers,
        &cookies,
    );
    let (stats, _) = store.insert_with_request(
        stored_entry("https://example.com/page", CacheScope::Public, "fr", vary),
        None,
        &fr_headers,
        &cookies,
    );

    // The en variant is evicted, but the fr variant still references the base
    assert_eq!(stats.size_evictions, 1);
    assert!(store
        .variants_by_base
        .contains_key("https://example.com/page"));
    assert!(store
        .lookup("https://example.com/page", &fr_headers, &cookies, None)
        .entry
        .is_some());
    assert!(store
        .lookup("https://example.com/page", &en_headers, &cookies, None)
        .entry
        .is_none());
}

#[test]
fn cleanup_expired_runs_at_most_once_per_second() {
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
            .expect("expected inserted entry");
        expired_entry.created_at = Instant::now() - Duration::from_secs(120);
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

    // First lookup scans and removes the expired entry.
    store
        .last_cleanup
        .store(0, std::sync::atomic::Ordering::Relaxed);
    let first = store.lookup("https://example.com/expired", &headers, &cookies, None);
    assert_eq!(first.stats.expired_evictions, 1);

    // A second lookup within the throttle window must not re-scan.
    let second = store.lookup("https://example.com/expired", &headers, &cookies, None);
    assert_eq!(second.stats.expired_evictions, 0);
}

#[test]
fn expired_entry_not_served_while_cleanup_throttled() {
    let store = CacheStore::new(4);
    let headers = HeaderMap::new();
    let cookies = AHashMap::default();

    // The insert runs the scan, so the throttle window starts now.
    store.insert_with_request(
        stored_entry(
            "https://example.com/page",
            CacheScope::Public,
            "body",
            VaryRule::default(),
        ),
        None,
        &headers,
        &cookies,
    );
    {
        let mut expired_entry = store
            .entries
            .get("https://example.com/page\nscope=public")
            .expect("expected inserted entry");
        expired_entry.created_at = Instant::now() - Duration::from_secs(120);
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

    // Lookup happens inside the throttle window: the expired entry is still
    // present in the cache, but the age checks skip it and it is not served.
    let outcome = store.lookup("https://example.com/page", &headers, &cookies, None);
    assert_eq!(outcome.stats.expired_evictions, 0);
    assert!(outcome.entry.is_none());
    assert!(store
        .entries
        .get("https://example.com/page\nscope=public")
        .is_some());
}
