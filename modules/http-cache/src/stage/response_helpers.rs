use std::convert::TryFrom;
use std::io;
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use ferron_core::pipeline::PipelineError;
use futures_util::stream::{self, StreamExt};
use http::header::{self, HeaderName, HeaderValue};
use http::{HeaderMap, Response};
use http_body::{Body as _, Frame};
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, BodyStream, Empty, Full, StreamBody};

use crate::lscache::{LS_CACHE, LS_CACHE_CONTROL, LS_COOKIE, LS_PURGE, LS_TAG, LS_VARY};
use crate::policy::CacheScope;
use crate::store::LookupEntry;

const CACHE_STATUS_HEADER: HeaderName = HeaderName::from_static("cache-status");

pub(super) enum CacheHeaderState<'a> {
    Hit { scope: CacheScope, age: Duration },
    StaleWhileRevalidate { scope: CacheScope, age: Duration },
    Revalidated,
    Miss { stored: bool, detail: &'a str },
    Bypass { detail: &'a str },
}

pub(super) enum CollectBodyOutcome {
    Complete(Option<Bytes>),
    Overflow {
        prefix: Bytes,
        remainder: UnsyncBoxBody<Bytes, io::Error>,
    },
}

pub(super) fn build_cached_response(
    entry: LookupEntry,
    head_only: bool,
    emit_ls_cache: bool,
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
    headers.remove(&LS_CACHE);
    headers.remove(header::AGE);
    headers.remove(CACHE_STATUS_HEADER);
    append_lsc_cookies_as_set_cookie(&mut headers, &entry.lsc_cookies);
    annotate_response_headers(
        &mut headers,
        CacheHeaderState::Hit {
            scope: entry.scope,
            age: entry.age,
        },
        emit_ls_cache,
    );

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

pub(super) fn annotate_response_headers(
    headers: &mut HeaderMap,
    state: CacheHeaderState<'_>,
    emit_ls_cache: bool,
) {
    if emit_ls_cache {
        headers.remove(&LS_CACHE);
    }
    headers.remove(CACHE_STATUS_HEADER);
    headers.remove(header::AGE);

    match state {
        CacheHeaderState::Hit { scope, age } => {
            if emit_ls_cache {
                let ls_value = if scope == CacheScope::Private {
                    "hit,private"
                } else {
                    "hit"
                };
                headers.insert(&LS_CACHE, HeaderValue::from_static(ls_value));
            }
            if let Ok(age_value) = HeaderValue::from_str(&age.as_secs().to_string()) {
                headers.insert(header::AGE, age_value);
            }
            let mut value = String::with_capacity(48 + scope.as_str().len());
            value.push_str("FerronCache; hit; detail=");
            value.push_str(scope.as_str());
            value.push_str("; age=");
            value.push_str(&age.as_secs().to_string());
            if let Ok(value) = HeaderValue::from_str(&value) {
                headers.insert(CACHE_STATUS_HEADER, value);
            }
        }
        CacheHeaderState::StaleWhileRevalidate { scope, age } => {
            if emit_ls_cache {
                let ls_value = if scope == CacheScope::Private {
                    "hit,private"
                } else {
                    "hit"
                };
                headers.insert(&LS_CACHE, HeaderValue::from_static(ls_value));
            }
            if let Ok(age_value) = HeaderValue::from_str(&age.as_secs().to_string()) {
                headers.insert(header::AGE, age_value);
            }
            let mut value = String::with_capacity(70 + scope.as_str().len());
            value.push_str("FerronCache; hit; detail=stale-while-revalidate,");
            value.push_str(scope.as_str());
            value.push_str("; age=");
            value.push_str(&age.as_secs().to_string());
            if let Ok(value) = HeaderValue::from_str(&value) {
                headers.insert(CACHE_STATUS_HEADER, value);
            }
        }
        CacheHeaderState::Revalidated => {
            if emit_ls_cache {
                headers.insert(&LS_CACHE, HeaderValue::from_static("hit"));
            }
            if let Ok(value) = HeaderValue::from_str("FerronCache; fwd=hit; detail=revalidated") {
                headers.insert(CACHE_STATUS_HEADER, value);
            }
        }
        CacheHeaderState::Miss { stored, detail } => {
            if emit_ls_cache {
                headers.insert(&LS_CACHE, HeaderValue::from_static("miss"));
            }
            let mut value = String::with_capacity(40 + detail.len());
            value.push_str("FerronCache; fwd=miss; stored=");
            value.push_str(if stored { "true" } else { "false" });
            value.push_str("; detail=");
            value.push_str(detail);
            if let Ok(value) = HeaderValue::from_str(&value) {
                headers.insert(CACHE_STATUS_HEADER, value);
            }
        }
        CacheHeaderState::Bypass { detail } => {
            if emit_ls_cache {
                headers.insert(&LS_CACHE, HeaderValue::from_static("bypass"));
            }
            let mut value = String::with_capacity(32 + detail.len());
            value.push_str("FerronCache; fwd=bypass; detail=");
            value.push_str(detail);
            if let Ok(value) = HeaderValue::from_str(&value) {
                headers.insert(CACHE_STATUS_HEADER, value);
            }
        }
    }
}

#[inline]
pub(super) fn strip_internal_headers(headers: &mut HeaderMap) {
    headers.remove(&LS_CACHE_CONTROL);
    headers.remove(&LS_TAG);
    headers.remove(&LS_PURGE);
    headers.remove(&LS_VARY);
    headers.remove(&LS_COOKIE);
    headers.remove(&LS_CACHE);
    headers.remove(CACHE_STATUS_HEADER);
}

#[inline]
pub(super) fn append_lsc_cookies_as_set_cookie(
    headers: &mut HeaderMap,
    lsc_cookies: &[HeaderValue],
) {
    headers.remove(&LS_COOKIE);
    for cookie in lsc_cookies {
        headers.append(header::SET_COOKIE, cookie.clone());
    }
}

pub(super) async fn collect_body_with_limit(
    body: Option<&mut UnsyncBoxBody<Bytes, io::Error>>,
    max_size: usize,
) -> Result<CollectBodyOutcome, PipelineError> {
    let Some(body) = body else {
        return Ok(CollectBodyOutcome::Complete(None));
    };
    let initial_capacity = body
        .size_hint()
        .upper()
        .and_then(|upper| usize::try_from(upper).ok())
        .map(|cap| cap.min(max_size))
        .unwrap_or(0);
    let mut buffer = BytesMut::with_capacity(initial_capacity);
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|error| PipelineError::custom(error.to_string()))?;
        if let Some(data) = frame.data_ref() {
            buffer.extend_from_slice(data);
            if buffer.len() > max_size {
                let remainder = std::mem::replace(
                    body,
                    Empty::<Bytes>::new()
                        .map_err(|error| match error {})
                        .boxed_unsync(),
                );
                return Ok(CollectBodyOutcome::Overflow {
                    prefix: buffer.freeze(),
                    remainder,
                });
            }
        }
    }

    Ok(CollectBodyOutcome::Complete(Some(buffer.freeze())))
}

pub(super) fn response_from_parts(
    parts: http::response::Parts,
    body: Bytes,
    head_only: bool,
) -> Result<Response<UnsyncBoxBody<Bytes, io::Error>>, PipelineError> {
    let body = if head_only {
        Empty::<Bytes>::new()
            .map_err(|error| match error {})
            .boxed_unsync()
    } else {
        Full::new(body)
            .map_err(|error: std::convert::Infallible| match error {})
            .boxed_unsync()
    };
    Ok(Response::from_parts(parts, body))
}

#[inline]
pub(super) fn response_from_streaming_parts(
    parts: http::response::Parts,
    prefix: Bytes,
    remainder: UnsyncBoxBody<Bytes, io::Error>,
) -> Result<Response<UnsyncBoxBody<Bytes, io::Error>>, PipelineError> {
    let prefix_stream = stream::once(async move { Ok(Frame::data(prefix)) });
    let chained = prefix_stream.chain(BodyStream::new(remainder));
    let body = StreamBody::new(chained).boxed_unsync();
    Ok(Response::from_parts(parts, body))
}
