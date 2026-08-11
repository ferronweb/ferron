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

mod basic;
mod coalesce;
mod persist;
mod update;

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
