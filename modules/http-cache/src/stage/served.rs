use std::io;

use bytes::Bytes;
use ferron_core::pipeline::PipelineError;
use http::header::{self, HeaderValue};
use http::Response;
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Empty, Full};

use crate::lscache::LS_CACHE;
use crate::store::{remove_hop_by_hop_headers, LookupEntry};

use super::response_helpers::{
    annotate_response_headers, append_lsc_cookies_as_set_cookie, CacheHeaderState,
    CACHE_STATUS_HEADER,
};

/// The state in which a cached entry is served to the client.
#[derive(Clone, Copy)]
pub(super) enum ServedState {
    Hit,
    StaleWhileRevalidate,
    Revalidated,
    StaleIfError,
}

/// Assemble the client response for a cached entry.
///
/// This is the single seam through which every served response passes: fresh
/// hits, singleflight-coalesced hits, stale-while-revalidate serves,
/// stale-if-error serves, and 304 revalidation rebuilds. It owns the header
/// hygiene, LSCookie rehydration, HEAD content-length fixup, and the
/// cache-status / X-LiteSpeed-Cache annotation.
pub(super) fn serve(
    entry: LookupEntry,
    state: ServedState,
    head_only: bool,
    emit_ls_headers: bool,
) -> Result<Response<UnsyncBoxBody<Bytes, io::Error>>, PipelineError> {
    let body_len = entry.body.as_ref().map(|body| body.len());
    let mut response = Response::new(if head_only {
        Empty::<Bytes>::new()
            .map_err(|error| match error {})
            .boxed_unsync()
    } else if let Some(body) = entry.body {
        Full::new(body)
            .map_err(|error: std::convert::Infallible| match error {})
            .boxed_unsync()
    } else {
        Empty::<Bytes>::new()
            .map_err(|error| match error {})
            .boxed_unsync()
    });
    *response.status_mut() = entry.status;
    let mut headers = entry.headers.clone();
    remove_hop_by_hop_headers(&mut headers);
    headers.remove(&LS_CACHE);
    headers.remove(header::AGE);
    headers.remove(CACHE_STATUS_HEADER);
    // Never replay a stored `Set-Cookie` verbatim: a cached hit must only
    // rehydrate cookies tracked separately in `lsc_cookies`.
    headers.remove(header::SET_COOKIE);
    if !matches!(state, ServedState::Revalidated) {
        append_lsc_cookies_as_set_cookie(&mut headers, &entry.lsc_cookies);
    }
    let annotation = match state {
        ServedState::Hit => CacheHeaderState::Hit {
            scope: entry.scope,
            age: entry.age,
        },
        ServedState::StaleWhileRevalidate => CacheHeaderState::StaleWhileRevalidate {
            scope: entry.scope,
            age: entry.age,
        },
        ServedState::StaleIfError => CacheHeaderState::StaleIfError {
            scope: entry.scope,
            age: entry.age,
        },
        ServedState::Revalidated => CacheHeaderState::Revalidated,
    };
    annotate_response_headers(&mut headers, annotation, emit_ls_headers);

    if head_only && !headers.contains_key(header::CONTENT_LENGTH) {
        if let Some(body_len) = body_len {
            let value = HeaderValue::from_str(&body_len.to_string())
                .map_err(|error| PipelineError::custom(error.to_string()))?;
            headers.insert(header::CONTENT_LENGTH, value);
        }
    }

    *response.headers_mut() = headers;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use http::{HeaderMap, HeaderName, HeaderValue, StatusCode};

    use crate::policy::CacheScope;

    use super::*;

    fn test_entry() -> LookupEntry {
        LookupEntry {
            scope: CacheScope::Public,
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            body: Some(Bytes::from_static(b"hello")),
            lsc_cookies: Vec::new(),
            age: Duration::from_secs(5),
            etag: None,
            last_modified: None,
            stale_if_error: None,
            must_revalidate: false,
            ttl: Duration::from_secs(60),
        }
    }

    #[tokio::test]
    async fn hit_response_uses_empty_body_for_head() {
        let response = serve(test_entry(), ServedState::Hit, true, false).unwrap();
        let collected = response.into_body().collect().await.unwrap().to_bytes();
        assert!(collected.is_empty());
    }

    #[tokio::test]
    async fn hit_response_annotates_cache_status() {
        let response = serve(test_entry(), ServedState::Hit, false, false).unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let status = response
            .headers()
            .get(CACHE_STATUS_HEADER)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(status.contains("hit"));
        let collected = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(collected, Bytes::from_static(b"hello"));
    }

    #[tokio::test]
    async fn stale_while_revalidate_response_annotates_stale() {
        let response = serve(
            test_entry(),
            ServedState::StaleWhileRevalidate,
            false,
            false,
        )
        .unwrap();
        let status = response
            .headers()
            .get(CACHE_STATUS_HEADER)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(status.contains("stale-while-revalidate"));
    }

    #[tokio::test]
    async fn stale_if_error_response_annotates_stale() {
        let response = serve(test_entry(), ServedState::StaleIfError, false, false).unwrap();
        let status = response
            .headers()
            .get(CACHE_STATUS_HEADER)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(status.contains("stale-if-error"));
        assert!(!status.contains("stale-while-revalidate"));
    }

    #[tokio::test]
    async fn revalidated_response_annotates_revalidated() {
        let response = serve(test_entry(), ServedState::Revalidated, false, false).unwrap();
        let status = response
            .headers()
            .get(CACHE_STATUS_HEADER)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(status.contains("revalidated"));
    }

    #[test]
    fn head_response_gets_content_length_from_stored_body() {
        let response = serve(test_entry(), ServedState::Hit, true, false).unwrap();
        assert_eq!(response.headers().get(header::CONTENT_LENGTH).unwrap(), "5");
    }

    #[test]
    fn serve_strips_internal_headers() {
        let mut entry = test_entry();
        entry
            .headers
            .insert(&LS_CACHE, HeaderValue::from_static("hit"));
        entry
            .headers
            .insert(header::AGE, HeaderValue::from_static("5"));
        entry.headers.insert(
            CACHE_STATUS_HEADER,
            HeaderValue::from_static("FerronCache; hit"),
        );
        let response = serve(entry, ServedState::Hit, false, false).unwrap();
        assert!(response.headers().get(&LS_CACHE).is_none());
        assert_eq!(response.headers().get(header::AGE).unwrap(), "5");
        assert_eq!(
            response.headers().get(CACHE_STATUS_HEADER).unwrap(),
            "FerronCache; hit; detail=public; age=5"
        );
    }

    #[test]
    fn serve_strips_hop_by_hop_headers_and_connection_named_fields() {
        let mut entry = test_entry();
        entry
            .headers
            .insert(header::CONNECTION, HeaderValue::from_static("X-Custom"));
        entry.headers.insert(
            "X-Custom".parse::<HeaderName>().unwrap(),
            HeaderValue::from_static("1"),
        );
        entry.headers.insert(
            header::TRANSFER_ENCODING,
            HeaderValue::from_static("chunked"),
        );
        entry
            .headers
            .insert(header::UPGRADE, HeaderValue::from_static("websocket"));
        let response = serve(entry, ServedState::Hit, false, false).unwrap();
        assert!(response.headers().get(header::CONNECTION).is_none());
        assert!(response.headers().get("X-Custom").is_none());
        assert!(response.headers().get(header::TRANSFER_ENCODING).is_none());
        assert!(response.headers().get(header::UPGRADE).is_none());
    }

    #[test]
    fn serve_never_replays_set_cookie_but_rehydrates_lsc_cookies() {
        let mut entry = test_entry();
        entry.headers.insert(
            header::SET_COOKIE,
            HeaderValue::from_static("origin_session=stale"),
        );
        entry.lsc_cookies = vec![HeaderValue::from_static("ferron_session=abc")];
        let response = serve(entry, ServedState::Hit, false, false).unwrap();
        let set_cookies: Vec<_> = response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|value| value.to_str().unwrap().to_string())
            .collect();
        assert_eq!(set_cookies, vec!["ferron_session=abc"]);
    }

    #[test]
    fn litespeed_header_emission_on_hit() {
        let response = serve(test_entry(), ServedState::Hit, false, true).unwrap();
        assert_eq!(response.headers().get(&LS_CACHE).unwrap(), "hit");
    }
}
