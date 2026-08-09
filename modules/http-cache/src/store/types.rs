use std::time::{Duration, Instant};

use bytes::Bytes;
use http::header::{HeaderName, HeaderValue};
use http::{HeaderMap, StatusCode};

use crate::lscache::ScopedTag;
use crate::policy::CacheScope;

#[derive(Clone, Debug)]
pub enum LookupHit {
    Fresh,
    StaleWhileRevalidate,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct VaryRule {
    pub header_names: Vec<HeaderName>,
    pub cookie_names: Vec<String>,
    pub value: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StoredVariant {
    pub scope: CacheScope,
    pub vary: VaryRule,
}

#[derive(Clone)]
pub struct StoredEntry {
    pub scope: CacheScope,
    pub base_key: String,
    pub vary: VaryRule,
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Option<Bytes>,
    pub lsc_cookies: Vec<HeaderValue>,
    pub created_at: Instant,
    pub ttl: Duration,
    pub access_at: u64,
    pub private_key: Option<String>,
    pub tags: Vec<ScopedTag>,
    pub purge_url: String,
    pub purge_host: String,
    pub etag: Option<HeaderValue>,
    pub last_modified: Option<HeaderValue>,
    pub stale_while_revalidate: Option<Duration>,
    pub stale_if_error: Option<Duration>,
    pub must_revalidate: bool,
}

#[derive(Clone)]
pub struct LookupEntry {
    pub scope: CacheScope,
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Option<Bytes>,
    pub lsc_cookies: Vec<HeaderValue>,
    pub age: Duration,
    pub etag: Option<HeaderValue>,
    pub last_modified: Option<HeaderValue>,
    pub stale_if_error: Option<Duration>,
    pub must_revalidate: bool,
    pub ttl: Duration,
}

#[derive(Default, Clone, Copy)]
pub struct StoreStats {
    pub size_evictions: usize,
    pub expired_evictions: usize,
    pub purged: usize,
}

/// Result of a store lookup: the matched entry, stats, current store size,
/// and whether variants exist for the base key without anything servable.
#[derive(Clone)]
pub struct LookupOutcome {
    pub entry: Option<(LookupEntry, String, LookupHit)>,
    pub stats: StoreStats,
    pub items: usize,
    pub had_expired: bool,
}
