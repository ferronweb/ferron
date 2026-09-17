use super::*;
use rustc_hash::FxHashMap;
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

    store.insert_with_request(entry, None, &headers, &cookies, &FxHashMap::default());

    let LookupOutcome { entry: lookup, .. } = store.lookup(
        "https://example.com/page",
        &headers,
        &cookies,
        None,
        &FxHashMap::default(),
    );
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

    store.insert_with_request(entry, None, &headers, &cookies, &FxHashMap::default());

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
    store.insert_with_request(entry, None, &headers, &cookies, &FxHashMap::default());

    let LookupOutcome { entry: lookup, .. } = store.lookup(
        "https://example.com/page",
        &headers,
        &cookies,
        None,
        &FxHashMap::default(),
    );
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
    store.insert_with_request(entry, None, &headers, &cookies, &FxHashMap::default());

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

    let LookupOutcome { entry: lookup, .. } = store.lookup(
        "https://example.com/page",
        &headers,
        &cookies,
        None,
        &FxHashMap::default(),
    );
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
    store.insert_with_request(entry, None, &headers, &cookies, &FxHashMap::default());

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
    } = store.lookup(
        "https://example.com/page",
        &headers,
        &cookies,
        None,
        &FxHashMap::default(),
    )
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
    std::sync::Arc::make_mut(&mut entry.headers)
        .append(CACHE_CONTROL, HeaderValue::from_static("max-age=999"));
    store.insert_with_request(entry, None, &headers, &cookies, &FxHashMap::default());

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

    let LookupOutcome { entry: lookup, .. } = store.lookup(
        "https://example.com/page",
        &headers,
        &cookies,
        None,
        &FxHashMap::default(),
    );
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
    store.insert_with_request(host_a, None, &headers, &cookies, &FxHashMap::default());

    let mut host_b = stored_entry(
        "https://b.example.com/page",
        CacheScope::Public,
        "b",
        VaryRule::default(),
    );
    host_b.purge_host = "b.example.com".to_string();
    store.insert_with_request(host_b, None, &headers, &cookies, &FxHashMap::default());

    let operations = vec![PurgeOperation {
        scope: CacheScope::Public,
        selectors: vec![PurgeSelector::All],
        stale: false,
    }];

    let (stats, len) = store.purge(&operations, None, Some("b.example.com"));
    assert_eq!(stats.purged, 1);
    assert_eq!(len, 1);
    assert!(store
        .lookup(
            "https://b.example.com/page",
            &headers,
            &cookies,
            None,
            &FxHashMap::default()
        )
        .entry
        .is_none());
    assert!(store
        .lookup(
            "https://a.example.com/page",
            &headers,
            &cookies,
            None,
            &FxHashMap::default()
        )
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
    store.insert_with_request(host_a, None, &headers, &cookies, &FxHashMap::default());

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
    store.insert_with_request(host_b, None, &headers, &cookies, &FxHashMap::default());

    let operations = vec![PurgeOperation {
        scope: CacheScope::Public,
        selectors: vec![PurgeSelector::Tag("v1".to_string())],
        stale: false,
    }];

    let (stats, len) = store.purge(&operations, None, Some("a.example.com"));
    assert_eq!(stats.purged, 1);
    assert_eq!(len, 1);
    assert!(store
        .lookup(
            "https://a.example.com/a",
            &headers,
            &cookies,
            None,
            &FxHashMap::default()
        )
        .entry
        .is_none());
    assert!(store
        .lookup(
            "https://b.example.com/b",
            &headers,
            &cookies,
            None,
            &FxHashMap::default()
        )
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
    store.insert_with_request(entry, None, &headers, &cookies, &FxHashMap::default());

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
    store.insert_with_request(entry1, None, &headers, &cookies, &FxHashMap::default());

    let entry2 = stored_entry(
        "https://example.com/b",
        CacheScope::Public,
        "b",
        VaryRule::default(),
    );
    let (stats, _) =
        store.insert_with_request(entry2, None, &headers, &cookies, &FxHashMap::default());

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
        no_vary: false,
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
        &FxHashMap::default(),
    );
    let (stats, _) = store.insert_with_request(
        stored_entry("https://example.com/page", CacheScope::Public, "fr", vary),
        None,
        &fr_headers,
        &cookies,
        &FxHashMap::default(),
    );

    // The en variant is evicted, but the fr variant still references the base
    assert_eq!(stats.size_evictions, 1);
    assert!(store
        .variants_by_base
        .contains_key("https://example.com/page"));
    assert!(store
        .lookup(
            "https://example.com/page",
            &fr_headers,
            &cookies,
            None,
            &FxHashMap::default()
        )
        .entry
        .is_some());
    assert!(store
        .lookup(
            "https://example.com/page",
            &en_headers,
            &cookies,
            None,
            &FxHashMap::default()
        )
        .entry
        .is_none());
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
        &FxHashMap::default(),
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
    let outcome = store.lookup(
        "https://example.com/page",
        &headers,
        &cookies,
        None,
        &FxHashMap::default(),
    );
    //assert_eq!(outcome.stats.expired_evictions, 0);
    assert!(outcome.entry.is_none());
    assert!(store
        .entries
        .get("https://example.com/page\nscope=public")
        .is_some());
}

#[test]
fn purge_stale_expires_entry_for_stale_serving() {
    let store = CacheStore::new(8);
    let headers = HeaderMap::new();
    let cookies = AHashMap::default();
    let no_vars = FxHashMap::default();

    let mut entry = stored_entry(
        "https://example.com/page",
        CacheScope::Public,
        "v1",
        VaryRule::default(),
    );
    entry.stale_while_revalidate = Some(Duration::from_secs(120));
    entry.tags = vec![ScopedTag {
        scope: CacheScope::Public,
        name: "stale-tag".to_string(),
    }];
    store.insert_with_request(entry, None, &headers, &cookies, &no_vars);

    let operations = vec![PurgeOperation {
        scope: CacheScope::Public,
        selectors: vec![PurgeSelector::Tag("stale-tag".to_string())],
        stale: true,
    }];
    let (stats, len) = store.purge_stale(&operations, None, None);
    assert_eq!(stats.purged, 1);
    // Soft purge retains the entry (unlike a hard purge).
    assert_eq!(len, 1);

    let LookupOutcome { entry: lookup, .. } = store.lookup(
        "https://example.com/page",
        &headers,
        &cookies,
        None,
        &no_vars,
    );
    let (_, _, hit) = lookup.expect("stale entry should still serve");
    assert!(matches!(hit, crate::store::LookupHit::StaleWhileRevalidate));
}

#[test]
fn purge_stale_without_window_behaves_like_miss() {
    let store = CacheStore::new(8);
    let headers = HeaderMap::new();
    let cookies = AHashMap::default();
    let no_vars = FxHashMap::default();

    // No stale-while-revalidate window: expiring can only miss.
    store.insert_with_request(
        stored_entry(
            "https://example.com/page",
            CacheScope::Public,
            "v1",
            VaryRule::default(),
        ),
        None,
        &headers,
        &cookies,
        &no_vars,
    );

    let operations = vec![PurgeOperation {
        scope: CacheScope::Public,
        selectors: vec![PurgeSelector::All],
        stale: true,
    }];
    let (stats, len) = store.purge_stale(&operations, None, None);
    assert_eq!(stats.purged, 1);
    assert_eq!(len, 1);

    let outcome = store.lookup(
        "https://example.com/page",
        &headers,
        &cookies,
        None,
        &no_vars,
    );
    assert!(outcome.entry.is_none());
    assert!(outcome.had_expired);
}

#[test]
fn vary_value_partitions_stored_variants() {
    let store = CacheStore::new(8);
    let headers = HeaderMap::new();
    let cookies = AHashMap::default();
    let vary = VaryRule {
        header_names: Vec::new(),
        cookie_names: Vec::new(),
        value: Some("device_class".to_string()),
        no_vary: false,
    };
    let mobile_vars: FxHashMap<String, String> =
        [("device_class".to_string(), "mobile".to_string())]
            .into_iter()
            .collect();
    let desktop_vars = FxHashMap::default();

    store.insert_with_request(
        stored_entry(
            "https://example.com/page",
            CacheScope::Public,
            "mobile-body",
            vary.clone(),
        ),
        None,
        &headers,
        &cookies,
        &mobile_vars,
    );
    // A request without the variable misses the mobile variant.
    assert!(store
        .lookup(
            "https://example.com/page",
            &headers,
            &cookies,
            None,
            &desktop_vars
        )
        .entry
        .is_none());

    store.insert_with_request(
        stored_entry(
            "https://example.com/page",
            CacheScope::Public,
            "desktop-body",
            vary,
        ),
        None,
        &headers,
        &cookies,
        &desktop_vars,
    );

    let mobile = store
        .lookup(
            "https://example.com/page",
            &headers,
            &cookies,
            None,
            &mobile_vars,
        )
        .entry
        .expect("mobile variant should hit");
    assert_eq!(
        mobile.0.body,
        Some(bytes::Bytes::from_static(b"mobile-body"))
    );
    let desktop = store
        .lookup(
            "https://example.com/page",
            &headers,
            &cookies,
            None,
            &desktop_vars,
        )
        .entry
        .expect("desktop variant should hit");
    assert_eq!(
        desktop.0.body,
        Some(bytes::Bytes::from_static(b"desktop-body"))
    );
}
