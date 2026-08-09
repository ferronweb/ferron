use http::{header, HeaderMap};

use crate::lscache::{PurgeOperation, PurgeSelector};
use crate::policy::CacheScope;

use super::types::StoredEntry;

pub(crate) fn entry_matches_purge(
    entry: &StoredEntry,
    operation: &PurgeOperation,
    current_private_key: Option<&str>,
    requesting_host: Option<&str>,
) -> bool {
    if entry.scope != operation.scope {
        return false;
    }

    // Cross-tenant isolation: when the purge carries a host (the resolved
    // vhost, or the zone's default host for host-ambiguous requests), it only
    // touches entries stored for that same host. Without a host (a shared
    // named/global zone with no Host header) the purge remains zone-wide.
    if let Some(host) = requesting_host {
        if entry.purge_host != host {
            return false;
        }
    }

    if operation.scope == CacheScope::Private
        && current_private_key.is_some()
        && entry.private_key.as_deref() != current_private_key
    {
        return false;
    }

    operation.selectors.iter().any(|selector| match selector {
        PurgeSelector::All => true,
        PurgeSelector::Url(url) => entry.purge_url == *url,
        PurgeSelector::UrlPath(path) => {
            let normalized_purge_url = entry
                .purge_url
                .split_once(['?', '#'])
                .map_or(entry.purge_url.as_str(), |(url, _)| url);
            normalized_purge_url == *path
        }
        PurgeSelector::Tag(tag) => entry
            .tags
            .iter()
            .any(|entry_tag| entry_tag.scope == operation.scope && entry_tag.name == *tag),
    })
}

/// Hop-by-hop headers a proxy must not forward or store (RFC 9110 §7.6.1).
///
/// A proxy consumes these headers and must not pass them on. `Connection` is
/// handled separately because it also names additional hop-by-hop fields.
const HOP_BY_HOP_HEADERS: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

/// Remove all hop-by-hop headers from `headers`, including any field named in
/// the `Connection` header.
#[inline]
pub fn remove_hop_by_hop_headers(headers: &mut HeaderMap) {
    let mut connection_names = Vec::new();
    for value in headers.get_all(header::CONNECTION) {
        let Some(text) = value.to_str().ok() else {
            continue;
        };
        for token in text.split(',') {
            let token = token.trim();
            if !token.is_empty() {
                connection_names.push(token.to_string());
            }
        }
    }
    for name in &connection_names {
        if let Ok(name) = http::header::HeaderName::from_bytes(name.as_bytes()) {
            headers.remove(name);
        }
    }
    for name in HOP_BY_HOP_HEADERS {
        headers.remove(http::header::HeaderName::from_static(name));
    }
}

/// Strip headers from an upstream response before storing it.
///
/// Removes hop-by-hop headers, the proxy-added `Age`, and `Set-Cookie`. The
/// origin's `Set-Cookie` must never be stored and replayed verbatim from a
/// cache entry, even for private responses: stale session credentials would
/// leak to whoever matches the entry later. LSCache cookie metadata (`LSC-Cookie`)
/// is stored separately and rehydrated on serve.
#[inline]
pub fn strip_store_headers(headers: &mut HeaderMap) {
    remove_hop_by_hop_headers(headers);
    headers.remove(header::AGE);
    headers.remove(header::SET_COOKIE);
}
