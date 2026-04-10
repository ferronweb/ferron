use std::collections::BTreeSet;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::{Bytes, BytesMut};
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::StageConstraint;
use ferron_http::{HttpContext, HttpResponse};
use ferron_observability::{
    Event, LogEvent, LogLevel, MetricAttributeValue, MetricEvent, MetricType, MetricValue,
};
use futures_util::stream::{self, StreamExt};
use http::header::{self, HeaderName, HeaderValue};
use http::{HeaderMap, Method, Response};
use http_body::Frame;
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, BodyStream, Empty, Full, StreamBody};
use rustc_hash::FxHashMap;
use typemap_rev::TypeMapKey;

use crate::config::{parse_cache_config, parse_max_entries, CacheConfig};
use crate::lscache::{
    collect_lsc_cookies, parse_litespeed_cache_control, parse_litespeed_purge,
    parse_litespeed_tags, parse_litespeed_vary, LS_CACHE, LS_CACHE_CONTROL, LS_COOKIE, LS_PURGE,
    LS_TAG, LS_VARY,
};
use crate::policy::{
    evaluate_response_policy, parse_request_policy, CacheScope, RequestCachePolicy,
};
use crate::store::{
    strip_store_headers, CacheStore, LookupEntry, StoreStats, StoredEntry, VaryRule,
};

const LOG_TARGET: &str = "ferron-http-cache";
const CACHE_STATUS_HEADER: HeaderName = HeaderName::from_static("cache-status");
const PRIVATE_COOKIE_NAMES: &[&str] = &["frontend", "phpsessid", "xf_session", "lsc_private"];

struct RequestStateKey;

impl TypeMapKey for RequestStateKey {
    type Value = RequestState;
}

struct RequestState {
    config: CacheConfig,
    base_key: String,
    request_headers: HeaderMap,
    request_cookies: FxHashMap<String, String>,
    private_key: Option<String>,
    purge_url: String,
    request_policy: RequestCachePolicy,
    has_authorization: bool,
    head_only: bool,
    lookup_result: LookupResult,
}

enum LookupResult {
    Hit,
    Miss { stats: StoreStats },
    Bypass,
}

enum CollectBodyOutcome {
    Complete(Bytes),
    Overflow {
        prefix: Bytes,
        remainder: UnsyncBoxBody<Bytes, io::Error>,
    },
}

/// Pipeline stage for HTTP response caching.
pub struct HttpCacheStage {
    store: Arc<CacheStore>,
}

impl HttpCacheStage {
    pub fn new() -> Self {
        Self {
            store: Arc::new(CacheStore::new(crate::config::DEFAULT_MAX_CACHE_ENTRIES)),
        }
    }

    fn emit_request_metric(
        &self,
        ctx: &HttpContext,
        result: &'static str,
        scope: Option<CacheScope>,
        items: usize,
    ) {
        let mut attrs = vec![(
            "ferron.cache.result",
            MetricAttributeValue::StaticStr(result),
        )];
        if let Some(scope) = scope {
            attrs.push((
                "ferron.cache.scope",
                MetricAttributeValue::StaticStr(scope.as_str()),
            ));
        }
        ctx.events.emit(Event::Metric(MetricEvent {
            name: "ferron.cache.requests",
            attributes: attrs,
            ty: MetricType::Counter,
            value: MetricValue::U64(1),
            unit: Some("{request}"),
            description: Some("Number of cache lookups handled by the HTTP cache."),
        }));
        ctx.events.emit(Event::Metric(MetricEvent {
            name: "ferron.cache.entries",
            attributes: vec![],
            ty: MetricType::Gauge,
            value: MetricValue::U64(items as u64),
            unit: Some("{entry}"),
            description: Some("Number of entries currently stored in the HTTP cache."),
        }));
    }

    fn emit_store_metric(&self, ctx: &HttpContext, scope: CacheScope) {
        ctx.events.emit(Event::Metric(MetricEvent {
            name: "ferron.cache.stores",
            attributes: vec![(
                "ferron.cache.scope",
                MetricAttributeValue::StaticStr(scope.as_str()),
            )],
            ty: MetricType::Counter,
            value: MetricValue::U64(1),
            unit: Some("{response}"),
            description: Some("Number of responses stored in the HTTP cache."),
        }));
    }

    fn emit_eviction_metrics(&self, ctx: &HttpContext, stats: StoreStats) {
        if stats.expired_evictions > 0 {
            ctx.events.emit(Event::Metric(MetricEvent {
                name: "ferron.cache.evictions",
                attributes: vec![(
                    "ferron.cache.reason",
                    MetricAttributeValue::StaticStr("expired"),
                )],
                ty: MetricType::Counter,
                value: MetricValue::U64(stats.expired_evictions as u64),
                unit: Some("{entry}"),
                description: Some("Number of cache entries evicted from the HTTP cache."),
            }));
        }
        if stats.size_evictions > 0 {
            ctx.events.emit(Event::Metric(MetricEvent {
                name: "ferron.cache.evictions",
                attributes: vec![(
                    "ferron.cache.reason",
                    MetricAttributeValue::StaticStr("size"),
                )],
                ty: MetricType::Counter,
                value: MetricValue::U64(stats.size_evictions as u64),
                unit: Some("{entry}"),
                description: Some("Number of cache entries evicted from the HTTP cache."),
            }));
        }
    }

    fn emit_purge_metric(&self, ctx: &HttpContext, scope: CacheScope, purged: usize, items: usize) {
        if purged == 0 {
            return;
        }
        ctx.events.emit(Event::Metric(MetricEvent {
            name: "ferron.cache.purges",
            attributes: vec![(
                "ferron.cache.scope",
                MetricAttributeValue::StaticStr(scope.as_str()),
            )],
            ty: MetricType::Counter,
            value: MetricValue::U64(purged as u64),
            unit: Some("{entry}"),
            description: Some("Number of cache entries purged via LSCache-compatible controls."),
        }));
        ctx.events.emit(Event::Metric(MetricEvent {
            name: "ferron.cache.entries",
            attributes: vec![],
            ty: MetricType::Gauge,
            value: MetricValue::U64(items as u64),
            unit: Some("{entry}"),
            description: Some("Number of entries currently stored in the HTTP cache."),
        }));
    }
}

impl Default for HttpCacheStage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait(?Send)]
impl Stage<HttpContext> for HttpCacheStage {
    fn name(&self) -> &str {
        "cache"
    }

    fn constraints(&self) -> Vec<StageConstraint> {
        vec![
            StageConstraint::After("https_redirect".to_string()),
            StageConstraint::After("rewrite".to_string()),
            StageConstraint::After("rate_limit".to_string()),
            StageConstraint::After("http_response".to_string()),
            StageConstraint::After("basicauth".to_string()),
            StageConstraint::Before("forward_proxy".to_string()),
            StageConstraint::Before("reverse_proxy".to_string()),
            StageConstraint::Before("static_file".to_string()),
        ]
    }

    fn is_applicable(
        &self,
        config: Option<&ferron_core::config::ServerConfigurationBlock>,
    ) -> bool {
        config.is_some_and(|config| config.has_directive("cache"))
    }

    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        let config = parse_cache_config(&ctx.configuration);
        self.store
            .set_max_entries(parse_max_entries(&ctx.configuration));

        if !config.enabled {
            return Ok(true);
        }

        let Some(request) = ctx.req.as_ref() else {
            return Ok(true);
        };

        let request_headers = request.headers().clone();
        let request_cookies = parse_cookies(&request_headers);
        let request_policy = parse_request_policy(&request_headers);
        let has_authorization = request_headers.contains_key(header::AUTHORIZATION);
        let purge_url = request
            .uri()
            .path_and_query()
            .map(|value| value.as_str().to_string())
            .unwrap_or_else(|| request.uri().path().to_string());
        let base_key = build_base_key(ctx.encrypted, &request_headers, request.uri());
        let private_key = Some(build_private_cache_key(
            &request_cookies,
            ctx.remote_address.ip(),
            ctx.auth_user.as_deref(),
        ));
        let head_only = request.method() == Method::HEAD;

        let method_cacheable = matches!(request.method(), &Method::GET | &Method::HEAD);
        let request_is_lookup_eligible = method_cacheable
            && !request_headers.contains_key(header::RANGE)
            && !request_headers.contains_key(header::UPGRADE)
            && request_policy.allow_lookup;

        let lookup_result = if request_is_lookup_eligible {
            let (lookup, stats, items) = self.store.lookup(
                &base_key,
                &request_headers,
                &request_cookies,
                private_key.as_deref(),
            );
            if let Some(entry) = lookup {
                let scope = entry.scope;
                self.emit_eviction_metrics(ctx, stats);
                self.emit_request_metric(ctx, "hit", Some(scope), items);
                ctx.res = Some(HttpResponse::Custom(build_cached_response(
                    entry, head_only,
                )?));
                LookupResult::Hit
            } else {
                self.emit_eviction_metrics(ctx, stats);
                LookupResult::Miss { stats }
            }
        } else {
            LookupResult::Bypass
        };

        let stop = matches!(lookup_result, LookupResult::Hit);
        ctx.extensions.insert::<RequestStateKey>(RequestState {
            config,
            base_key,
            request_headers,
            request_cookies,
            private_key,
            purge_url,
            request_policy,
            has_authorization,
            head_only,
            lookup_result,
        });

        Ok(!stop)
    }

    async fn run_inverse(&self, ctx: &mut HttpContext) -> Result<(), PipelineError> {
        let Some(state) = ctx.extensions.remove::<RequestStateKey>() else {
            return Ok(());
        };

        match state.lookup_result {
            LookupResult::Hit => return Ok(()),
            LookupResult::Miss { stats } => self.emit_eviction_metrics(ctx, stats),
            LookupResult::Bypass => {}
        }

        let response = match ctx.res.take() {
            Some(HttpResponse::Custom(response)) => response,
            other => {
                ctx.res = other;
                return Ok(());
            }
        };

        let mut purge_scope = None;
        let purge_ops = parse_litespeed_purge(response.headers());
        if purge_ops.iter().any(|operation| operation.stale) {
            ctx.events.emit(Event::Log(LogEvent {
                level: LogLevel::Debug,
                target: LOG_TARGET,
                message:
                    "Ignoring unsupported LSCache stale purge marker and performing a hard purge"
                        .to_string(),
            }));
        }
        if !purge_ops.is_empty() {
            let (stats, items) = self.store.purge(&purge_ops, state.private_key.as_deref());
            for operation in &purge_ops {
                purge_scope = Some(operation.scope);
                self.emit_purge_metric(ctx, operation.scope, stats.purged, items);
            }
            if stats.purged > 0 {
                ctx.events.emit(Event::Log(LogEvent {
                    level: LogLevel::Debug,
                    target: LOG_TARGET,
                    message: format!(
                        "Purged {} cache entrie(s) via LSCache controls",
                        stats.purged
                    ),
                }));
            }
        }

        let ls_control = parse_litespeed_cache_control(response.headers());
        let ls_vary = if ls_control.as_ref().is_some_and(|control| control.no_vary) {
            crate::lscache::LiteSpeedVary::default()
        } else {
            parse_litespeed_vary(response.headers())
        };
        let has_unsupported_vary_value = ls_vary.value.is_some();
        if has_unsupported_vary_value {
            ctx.events.emit(Event::Log(LogEvent {
                level: LogLevel::Debug,
                target: LOG_TARGET,
                message:
                    "Skipping cache store because X-LiteSpeed-Vary: value=... is not supported yet"
                        .to_string(),
            }));
        }
        let has_set_cookie = response.headers().contains_key(header::SET_COOKIE);
        let decision = if !state.request_policy.allow_store
            || (matches!(state.lookup_result, LookupResult::Bypass)
                && state.request_policy.allow_lookup)
            || state.head_only
            || has_unsupported_vary_value
        {
            crate::policy::ResponseCacheDecision {
                store: false,
                scope: None,
                ttl: None,
                reason: if state.head_only {
                    "head-no-store"
                } else if has_unsupported_vary_value {
                    "unsupported-litespeed-vary-value"
                } else {
                    state.request_policy.reason
                },
            }
        } else {
            evaluate_response_policy(
                response.status(),
                response.headers(),
                state.has_authorization,
                has_set_cookie,
                ls_control.as_ref(),
                state.config.litespeed_override_cache_control,
            )
        };

        let vary_rule = build_vary_rule(response.headers(), &state.config, &ls_vary)?;
        let lsc_cookies = collect_lsc_cookies(response.headers());
        let mut response = response;

        if decision.store && vary_rule.is_some() {
            let scope = decision.scope.expect("scope must be set when storing");
            let tags = parse_litespeed_tags(response.headers(), scope);
            let (mut parts, mut body) = response.into_parts();
            parts.extensions.clear(); // Prevent zerocopy from interfering with cache
            strip_internal_headers(&mut parts.headers);
            append_lsc_cookies_as_set_cookie(&mut parts.headers, &lsc_cookies);
            let body_result =
                collect_body_with_limit(&mut body, state.config.max_response_size).await?;

            match body_result {
                CollectBodyOutcome::Complete(body_bytes) => {
                    let mut outgoing_response =
                        response_from_parts(parts, body_bytes.clone(), state.head_only)?;
                    let mut stored_headers = outgoing_response.headers().clone();
                    for header_name in &state.config.ignored_store_headers {
                        stored_headers.remove(header_name);
                    }
                    strip_store_headers(&mut stored_headers);
                    let stored_entry = StoredEntry {
                        scope,
                        base_key: state.base_key.clone(),
                        #[allow(clippy::unnecessary_unwrap)]
                        vary: vary_rule.expect("vary rule must exist"),
                        status: outgoing_response.status(),
                        headers: stored_headers,
                        body: body_bytes,
                        lsc_cookies: lsc_cookies.clone(),
                        created_at: std::time::Instant::now(),
                        ttl: decision.ttl.unwrap_or_else(|| Duration::from_secs(0)),
                        access_at: 0,
                        private_key: None,
                        tags,
                        purge_url: state.purge_url,
                    };
                    let (stats, items) = self.store.insert_with_request(
                        stored_entry,
                        state.private_key.as_deref(),
                        &state.request_headers,
                        &state.request_cookies,
                    );
                    self.emit_eviction_metrics(ctx, stats);
                    self.emit_store_metric(ctx, scope);
                    annotate_response_headers(
                        outgoing_response.headers_mut(),
                        CacheHeaderState::Miss {
                            stored: true,
                            detail: decision.reason,
                        },
                    );
                    self.emit_request_metric(ctx, "miss", Some(scope), items);
                    ctx.res = Some(HttpResponse::Custom(outgoing_response));
                }
                CollectBodyOutcome::Overflow { prefix, remainder } => {
                    ctx.events.emit(Event::Log(LogEvent {
                        level: LogLevel::Debug,
                        target: LOG_TARGET,
                        message: "Skipping cache store because the response body exceeded cache.max_response_size".to_string(),
                    }));
                    let mut response = response_from_streaming_parts(parts, prefix, remainder)?;
                    annotate_response_headers(
                        response.headers_mut(),
                        CacheHeaderState::Miss {
                            stored: false,
                            detail: "response-too-large",
                        },
                    );
                    self.emit_request_metric(ctx, "miss", None, self.store.len());
                    ctx.res = Some(HttpResponse::Custom(response));
                }
            }
        } else {
            strip_internal_headers(response.headers_mut());
            append_lsc_cookies_as_set_cookie(response.headers_mut(), &lsc_cookies);
            annotate_response_headers(
                response.headers_mut(),
                if matches!(state.lookup_result, LookupResult::Bypass) {
                    CacheHeaderState::Bypass {
                        detail: decision.reason,
                    }
                } else {
                    CacheHeaderState::Miss {
                        stored: false,
                        detail: decision.reason,
                    }
                },
            );
            let result = if matches!(state.lookup_result, LookupResult::Bypass) {
                "bypass"
            } else {
                "miss"
            };
            self.emit_request_metric(
                ctx,
                result,
                purge_scope.or(decision.scope),
                self.store.len(),
            );
            ctx.res = Some(HttpResponse::Custom(response));
        }

        Ok(())
    }
}

enum CacheHeaderState<'a> {
    Hit { scope: CacheScope, age: Duration },
    Miss { stored: bool, detail: &'a str },
    Bypass { detail: &'a str },
}

fn build_cached_response(
    entry: LookupEntry,
    head_only: bool,
) -> Result<Response<UnsyncBoxBody<Bytes, io::Error>>, PipelineError> {
    let mut builder = Response::builder().status(entry.status);
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
    );

    if head_only && !headers.contains_key(header::CONTENT_LENGTH) {
        let value = HeaderValue::from_str(&entry.body.len().to_string())
            .map_err(|error| PipelineError::custom(error.to_string()))?;
        headers.insert(header::CONTENT_LENGTH, value);
    }

    for (name, value) in &headers {
        builder = builder.header(name, value);
    }

    let body = if head_only {
        Empty::<Bytes>::new()
            .map_err(|error| match error {})
            .boxed_unsync()
    } else {
        Full::new(entry.body)
            .map_err(|error: std::convert::Infallible| match error {})
            .boxed_unsync()
    };

    builder
        .body(body)
        .map_err(|error| PipelineError::custom(error.to_string()))
}

fn build_base_key(encrypted: bool, headers: &HeaderMap, uri: &http::Uri) -> String {
    let scheme = if encrypted { "https" } else { "http" };
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let path_and_query = uri
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or("/");
    format!("{scheme}://{host}{path_and_query}")
}

fn parse_cookies(headers: &HeaderMap) -> FxHashMap<String, String> {
    let mut cookies = FxHashMap::default();
    for value in headers.get_all(header::COOKIE) {
        let Some(text) = value.to_str().ok() else {
            continue;
        };
        for cookie in text.split(';') {
            let Some((name, value)) = cookie.split_once('=') else {
                continue;
            };
            let name = name.trim();
            let value = value.trim();
            if !name.is_empty() {
                cookies.insert(name.to_string(), value.to_string());
            }
        }
    }
    cookies
}

fn build_private_cache_key(
    cookies: &FxHashMap<String, String>,
    remote_ip: std::net::IpAddr,
    auth_user: Option<&str>,
) -> String {
    let mut components = BTreeSet::new();
    components.insert(format!("ip={remote_ip}"));
    if let Some(auth_user) = auth_user {
        components.insert(format!("auth={auth_user}"));
    }

    let mut matched_private_cookie = false;
    for (name, value) in cookies {
        let lower = name.to_ascii_lowercase();
        let is_private = PRIVATE_COOKIE_NAMES.contains(&lower.as_str())
            || lower.starts_with("wp_woocommerce_session_");
        if is_private && value.len() >= 16 {
            matched_private_cookie = true;
            components.insert(format!("cookie:{name}={value}"));
        }
    }

    if !matched_private_cookie {
        for (name, value) in cookies {
            components.insert(format!("cookie:{name}={value}"));
        }
    }

    components.into_iter().collect::<Vec<_>>().join("&")
}

fn build_vary_rule(
    headers: &HeaderMap,
    config: &CacheConfig,
    ls_vary: &crate::lscache::LiteSpeedVary,
) -> Result<Option<VaryRule>, PipelineError> {
    let mut header_names = config.vary_headers.clone();
    for value in headers.get_all(header::VARY) {
        let Some(text) = value.to_str().ok() else {
            continue;
        };
        for token in text.split(',') {
            let token = token.trim();
            if token == "*" {
                return Ok(None);
            }
            if token.is_empty() {
                continue;
            }
            let name = HeaderName::from_bytes(token.as_bytes())
                .map_err(|error| PipelineError::custom(error.to_string()))?;
            if !header_names.contains(&name) {
                header_names.push(name);
            }
        }
    }
    header_names.sort_by(|left, right| left.as_str().cmp(right.as_str()));

    let mut cookie_names = ls_vary.cookies.clone();
    cookie_names.sort_unstable();

    Ok(Some(VaryRule {
        header_names,
        cookie_names,
        value: None,
    }))
}

async fn collect_body_with_limit(
    body: &mut UnsyncBoxBody<Bytes, io::Error>,
    max_size: usize,
) -> Result<CollectBodyOutcome, PipelineError> {
    let mut buffer = BytesMut::new();
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

    Ok(CollectBodyOutcome::Complete(buffer.freeze()))
}

fn response_from_parts(
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

fn response_from_streaming_parts(
    parts: http::response::Parts,
    prefix: Bytes,
    remainder: UnsyncBoxBody<Bytes, io::Error>,
) -> Result<Response<UnsyncBoxBody<Bytes, io::Error>>, PipelineError> {
    let prefix_stream = stream::once(async move { Ok(Frame::data(prefix)) });
    let chained = prefix_stream.chain(BodyStream::new(remainder));
    let body = StreamBody::new(chained).boxed_unsync();
    Ok(Response::from_parts(parts, body))
}

fn annotate_response_headers(headers: &mut HeaderMap, state: CacheHeaderState<'_>) {
    headers.remove(&LS_CACHE);
    headers.remove(CACHE_STATUS_HEADER);
    headers.remove(header::AGE);

    match state {
        CacheHeaderState::Hit { scope, age } => {
            let ls_value = if scope == CacheScope::Private {
                "hit,private"
            } else {
                "hit"
            };
            headers.insert(&LS_CACHE, HeaderValue::from_static(ls_value));
            if let Ok(age_value) = HeaderValue::from_str(&age.as_secs().to_string()) {
                headers.insert(header::AGE, age_value);
            }
            if let Ok(value) = HeaderValue::from_str(&format!(
                "FerronCache; hit; detail={}; age={}",
                scope.as_str(),
                age.as_secs()
            )) {
                headers.insert(CACHE_STATUS_HEADER, value);
            }
        }
        CacheHeaderState::Miss { stored, detail } => {
            headers.insert(&LS_CACHE, HeaderValue::from_static("miss"));
            if let Ok(value) = HeaderValue::from_str(&format!(
                "FerronCache; fwd=miss; stored={stored}; detail={detail}"
            )) {
                headers.insert(CACHE_STATUS_HEADER, value);
            }
        }
        CacheHeaderState::Bypass { detail } => {
            headers.insert(&LS_CACHE, HeaderValue::from_static("bypass"));
            if let Ok(value) =
                HeaderValue::from_str(&format!("FerronCache; fwd=bypass; detail={detail}"))
            {
                headers.insert(CACHE_STATUS_HEADER, value);
            }
        }
    }
}

fn strip_internal_headers(headers: &mut HeaderMap) {
    headers.remove(&LS_CACHE_CONTROL);
    headers.remove(&LS_TAG);
    headers.remove(&LS_PURGE);
    headers.remove(&LS_VARY);
    headers.remove(&LS_COOKIE);
    headers.remove(&LS_CACHE);
    headers.remove(CACHE_STATUS_HEADER);
}

fn append_lsc_cookies_as_set_cookie(headers: &mut HeaderMap, lsc_cookies: &[HeaderValue]) {
    headers.remove(&LS_COOKIE);
    for cookie in lsc_cookies {
        headers.append(header::SET_COOKIE, cookie.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::config::layer::LayeredConfiguration;
    use ferron_observability::CompositeEventSink;
    use http::Request;
    use std::net::SocketAddr;

    fn test_context(path: &str) -> HttpContext {
        let request = Request::builder()
            .uri(path)
            .header(header::HOST, "example.com")
            .body(
                Empty::<Bytes>::new()
                    .map_err(|error: std::convert::Infallible| match error {})
                    .boxed_unsync(),
            )
            .unwrap();

        HttpContext {
            req: Some(request),
            res: None,
            events: CompositeEventSink::new(Vec::new()),
            configuration: LayeredConfiguration::default(),
            hostname: Some("example.com".to_string()),
            variables: FxHashMap::default(),
            previous_error: None,
            original_uri: None,
            encrypted: true,
            local_address: "127.0.0.1:443".parse::<SocketAddr>().unwrap(),
            remote_address: "127.0.0.2:12345".parse::<SocketAddr>().unwrap(),
            auth_user: None,
            https_port: Some(443),
            extensions: typemap_rev::TypeMap::new(),
        }
    }

    #[test]
    fn parses_private_key_from_cookies() {
        let mut cookies = FxHashMap::default();
        cookies.insert("PHPSESSID".to_string(), "1234567890abcdef".to_string());
        let key = build_private_cache_key(&cookies, "127.0.0.1".parse().unwrap(), Some("user"));
        assert!(key.contains("auth=user"));
        assert!(key.contains("cookie:PHPSESSID=1234567890abcdef"));
    }

    #[tokio::test]
    async fn hit_response_uses_empty_body_for_head() {
        let entry = LookupEntry {
            scope: CacheScope::Public,
            status: http::StatusCode::OK,
            headers: HeaderMap::new(),
            body: Bytes::from_static(b"hello"),
            lsc_cookies: Vec::new(),
            age: Duration::from_secs(5),
        };
        let response = build_cached_response(entry, true).unwrap();
        let collected = response.into_body().collect().await.unwrap().to_bytes();
        assert!(collected.is_empty());
    }

    #[test]
    fn base_key_uses_scheme_host_and_path() {
        let ctx = test_context("/test?q=1");
        let request = ctx.req.as_ref().unwrap();
        let key = build_base_key(ctx.encrypted, request.headers(), request.uri());
        assert_eq!(key, "https://example.com/test?q=1");
    }
}
