use std::net::IpAddr;

use cidr::IpCidr;
use http::{header, HeaderMap, HeaderValue, Method};

use crate::config::{has_host_max_entries, CacheConfig, CacheZoneId};

use super::purge::PURGE_SECRET_HEADER;

/// Current configuration generation, bumped on every config reload.
#[inline]
pub(super) fn active_config_generation() -> u64 {
    ferron_core::admin::ADMIN_METRICS
        .reload_metrics
        .read()
        .active_generation
}

/// Whether a fresh cached representation satisfies the client's conditional
/// request headers, so the cache can answer `304 Not Modified` locally
/// without an upstream round trip.
///
/// Per RFC 9110 §13.1.1 and §13.1.3: `If-None-Match` takes precedence over
/// `If-Modified-Since`, and both use weak validator comparison for GET and
/// HEAD. When `If-None-Match` is present but does not match, the cache serves
/// the full representation instead of evaluating `If-Modified-Since`.
pub(super) fn client_conditionals_indicate_not_modified(
    method: &Method,
    request_headers: &HeaderMap,
    etag: Option<&HeaderValue>,
    last_modified: Option<&HeaderValue>,
) -> bool {
    if let Some(if_none_match) = request_headers.get(header::IF_NONE_MATCH) {
        let Ok(value) = if_none_match.to_str() else {
            return false;
        };
        let Some(etag) = etag else {
            return false;
        };
        let Ok(etag) = etag.to_str() else {
            return false;
        };
        return if value.trim() == "*" {
            true
        } else {
            value.split(',').any(|candidate| {
                let candidate = candidate.trim();
                !candidate.is_empty() && weak_etag_eq(candidate, etag)
            })
        };
    }

    if method == Method::GET || method == Method::HEAD {
        if let Some(if_modified_since) = request_headers.get(header::IF_MODIFIED_SINCE) {
            if let Some(last_modified) = last_modified {
                if let (Ok(if_modified_since), Ok(last_modified)) =
                    (if_modified_since.to_str(), last_modified.to_str())
                {
                    if let (Ok(since), Ok(modified)) = (
                        httpdate::parse_http_date(if_modified_since),
                        httpdate::parse_http_date(last_modified),
                    ) {
                        return modified <= since;
                    }
                }
            }
        }
    }
    false
}

/// RFC 9110 §8.8.3.2 weak entity-tag comparison: the `W/` prefix is ignored,
/// and the opaque-tags must match character-for-character.
pub(super) fn weak_etag_eq(client_etag: &str, stored_etag: &str) -> bool {
    let client_etag = client_etag.strip_prefix("W/").unwrap_or(client_etag);
    let stored_etag = stored_etag.strip_prefix("W/").unwrap_or(stored_etag);
    client_etag == stored_etag
}

/// Resolve the cache zone ID for a request.
pub(super) fn resolve_zone_id(
    hostname: &Option<String>,
    config: &CacheConfig,
    configuration: &ferron_core::config::layer::LayeredConfiguration,
) -> CacheZoneId {
    if let Some(ref zone) = config.zone {
        zone.clone()
    } else if has_host_max_entries(configuration) {
        CacheZoneId::Host(hostname.clone().unwrap_or_else(|| "_default".to_string()))
    } else if crate::config::has_global_zone(configuration) {
        CacheZoneId::Global
    } else {
        CacheZoneId::Host(hostname.clone().unwrap_or_else(|| "_default".to_string()))
    }
}

/// The host associated with a cache entry or purge request.
///
/// Prefers the resolved vhost. When the request is host-ambiguous, falls back
/// to the zone's own host for per-host zones so a host guard still applies;
/// shared named/global zones without a host resolve to an empty value, which
/// never matches a populated host.
pub(super) fn entry_host(hostname: &Option<String>, zone_id: &CacheZoneId) -> Option<String> {
    hostname.clone().or_else(|| match zone_id {
        CacheZoneId::Host(host) => Some(host.clone()),
        CacheZoneId::Named(_) | CacheZoneId::Global => None,
    })
}

/// Whether a `X-Purge-Source: propagation` purge proves knowledge of the
/// configured shared secret.
///
/// The secret is required: when none is configured, a propagation claim is
/// indistinguishable from a replay and is rejected. Comparison is
/// constant-time.
#[inline]
pub(super) fn propagation_secret_verified(
    request_headers: &HeaderMap,
    shared_secret: Option<&str>,
) -> bool {
    use subtle::ConstantTimeEq;

    let Some(configured) = shared_secret else {
        return false;
    };
    let Some(received) = request_headers
        .get(&PURGE_SECRET_HEADER)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    configured.as_bytes().ct_eq(received.as_bytes()).into()
}

/// Whether a `PURGE` request is authorized to invalidate this cache.
///
/// An allow-listed client IP always authorizes a purge. Otherwise the request
/// must carry an authenticated user (`ctx.auth_user`) **and** a `basic_auth`
/// block must be in scope for the request. Requiring the in-scope `basic_auth`
/// block prevents a user authenticated by a foreign host's `basic_auth` from
/// purging a host that does not own those credentials.
#[inline]
pub(super) fn purge_allowed(
    remote_ip: IpAddr,
    purge_allowed_ips: &[IpCidr],
    has_basic_auth_in_scope: bool,
    auth_user: Option<&str>,
) -> bool {
    if purge_allowed_ips
        .iter()
        .any(|cidr| cidr.contains(&remote_ip))
    {
        return true;
    }
    auth_user.is_some() && has_basic_auth_in_scope
}
