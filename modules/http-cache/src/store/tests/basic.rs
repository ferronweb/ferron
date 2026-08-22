use super::*;
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
        stats: _,
        items: len,
        had_expired,
    } = store.lookup(base_key, &headers, &cookies, None);
    let (lookup, _key, _hit) = lookup.expect("expected cache hit");
    //assert_eq!(stats.expired_evictions, 0);
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

    strip_store_headers(&mut headers);

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
fn strip_store_headers_always_removes_set_cookie() {
    let mut headers = HeaderMap::new();
    headers.insert(SET_COOKIE, HeaderValue::from_static("session=abc"));
    strip_store_headers(&mut headers);
    assert!(!headers.contains_key(SET_COOKIE));
}

#[test]
fn stored_entry_keeps_lsc_cookies_but_drops_origin_set_cookie() {
    let store = CacheStore::new(4);
    let base_key = "https://example.com/private";
    let headers = HeaderMap::new();
    let cookies = AHashMap::default();

    // Origin sends a session cookie plus LSCache cookie metadata.
    let mut upstream_headers = HeaderMap::new();
    upstream_headers.insert(SET_COOKIE, HeaderValue::from_static("session=abc"));
    upstream_headers.insert(
        HeaderName::from_static("lsc-cookie"),
        HeaderValue::from_static("lsc_session=xyz"),
    );
    let lsc_cookies = crate::lscache::collect_lsc_cookies(&upstream_headers);
    strip_store_headers(&mut upstream_headers);

    let mut entry = stored_entry(
        base_key,
        CacheScope::Private,
        "cached-body",
        VaryRule::default(),
    );
    entry.headers = upstream_headers;
    entry.lsc_cookies = lsc_cookies;
    store.insert_with_request(entry, Some("user=1"), &headers, &cookies);

    let LookupOutcome { entry: lookup, .. } =
        store.lookup(base_key, &headers, &cookies, Some("user=1"));
    let (lookup, _, _) = lookup.expect("expected private cache hit");
    assert!(!lookup.headers.contains_key(SET_COOKIE));
    assert_eq!(lookup.lsc_cookies.len(), 1);
    assert_eq!(lookup.lsc_cookies[0].to_str().unwrap(), "lsc_session=xyz");
}

#[test]
fn had_expired_when_variants_exist_but_no_request_matches() {
    let store = CacheStore::new(4);
    let vary = VaryRule {
        header_names: vec![HeaderName::from_static("accept-language")],
        cookie_names: Vec::new(),
        value: None,
    };
    let en_headers = request_headers(&[(&HeaderName::from_static("accept-language"), "en-US")]);
    let fr_headers = request_headers(&[(&HeaderName::from_static("accept-language"), "fr-FR")]);
    let cookies = AHashMap::default();

    store.insert_with_request(
        stored_entry("https://example.com/vary", CacheScope::Public, "en", vary),
        None,
        &en_headers,
        &cookies,
    );

    // A request whose headers match no stored variant still reports the base
    // as expired-so-fetch, because variants exist for it.
    let LookupOutcome { had_expired, .. } =
        store.lookup("https://example.com/vary", &fr_headers, &cookies, None);
    assert!(had_expired);
}
