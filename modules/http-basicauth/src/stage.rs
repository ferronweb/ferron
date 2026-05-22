//! HTTP Basic Authentication pipeline stage.
//!
//! Intercepts requests and validates the `Authorization: Basic` header against
//! configured users. Returns 401 for regular requests and 407 for CONNECT
//! requests when authentication fails.

use std::sync::{Arc, LazyLock};

use dashmap::DashMap;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::StageConstraint;
use ferron_http::abuse::{get_global_abuse_recorder, AbuseEvent, AbuseEventType};
use ferron_http::{HttpContext, HttpResponse};
use ferron_observability::{Event, LogEvent, LogLevel};
use http::{HeaderMap, HeaderValue, Method};
use tokio::sync::Semaphore;

use crate::brute_force::{BruteForceConfig, BruteForceEngine};
use crate::config::parse_basicauth_config;

/// A fake hash used to thwart user enumeration attacks.
/// This hash is a valid, hard-coded Argon2 hash for an empty password.
const FAKE_HASH: &str = "$argon2id$v=19$m=19456,t=2,\
p=1$xvAbcK77AZqOdJtrS1LqWA$bd5QzFMwzDFGZ5I7FAX3roi9Gw2m/nFo3Ivw/W25f50";

pub(crate) static GLOBAL_CONCURRENCY_SEMAPHORE: LazyLock<
    tokio::sync::RwLock<Option<Arc<Semaphore>>>,
> = LazyLock::new(|| tokio::sync::RwLock::new(None));

/// Pipeline stage that enforces HTTP Basic Authentication.
pub struct BasicAuthStage {
    /// A map of cached brute force engines
    engines: DashMap<BruteForceConfig, BruteForceEngine>,
}

impl BasicAuthStage {
    /// Create a new basic auth stage with the shared engine.
    pub fn new() -> Self {
        Self {
            engines: DashMap::new(),
        }
    }

    /// Extract the username from a Basic auth header value.
    ///
    /// The value is expected to be `Basic <base64(username:password)>`.
    fn parse_basic_auth_header(value: &str) -> Option<(String, String)> {
        let value = value
            .trim_start_matches("Basic ")
            .trim_start_matches("basic ");
        let decoded =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, value).ok()?;
        let credentials = String::from_utf8(decoded).ok()?;

        let colon_pos = credentials.find(':')?;
        let username = credentials[..colon_pos].to_string();
        let password = credentials[colon_pos + 1..].to_string();

        Some((username, password))
    }

    /// Verify a password against a stored hash.
    async fn verify_password(plain: &str, hash: &str) -> bool {
        let plain = plain.to_string();
        let hash = hash.to_string();
        let sem = GLOBAL_CONCURRENCY_SEMAPHORE.read().await.clone();
        let _permit = if let Some(sem) = sem {
            sem.acquire_owned().await.ok()
        } else {
            None
        };
        let result =
            vibeio::spawn_blocking(move || password_auth::verify_password(&plain, &hash).is_ok())
                .await
                .unwrap_or(false);
        drop(_permit);
        result
    }

    /// Build an authentication challenge response.
    ///
    /// For CONNECT requests, returns 407 Proxy Authentication Required.
    /// For all other requests, returns 401 Unauthorized.
    fn make_auth_challenge_response(ctx: &HttpContext, realm: &str) -> HttpResponse {
        let is_connect = ctx
            .req
            .as_ref()
            .map(|r| r.method() == Method::CONNECT)
            .unwrap_or(false);

        let status = if is_connect { 407 } else { 401 };
        let header_name = if is_connect {
            http::header::PROXY_AUTHENTICATE
        } else {
            http::header::WWW_AUTHENTICATE
        };

        let challenge = format!("Basic realm=\"{realm}\", charset=\"UTF-8\"");
        let mut headers = HeaderMap::new();
        headers.insert(
            header_name,
            HeaderValue::from_str(&challenge)
                .expect("challenge value should be valid header value"),
        );

        HttpResponse::BuiltinError(status, Some(headers))
    }

    /// Build a lockout response (IP temporarily locked due to brute-force protection).
    fn make_lockout_response(ctx: &HttpContext, engine: &BruteForceEngine) -> HttpResponse {
        let retry_after = engine.retry_after(ctx.remote_address.ip());
        let mut header_map = HeaderMap::new();
        if let Some(duration) = retry_after {
            header_map.insert(
                http::header::RETRY_AFTER,
                HeaderValue::from_str(&duration.as_secs().to_string())
                    .expect("retry-after value should be valid header value"),
            );
        }

        HttpResponse::BuiltinError(429, Some(header_map))
    }
}

#[async_trait::async_trait(?Send)]
impl Stage<HttpContext> for BasicAuthStage {
    fn name(&self) -> &str {
        "basicauth"
    }

    fn constraints(&self) -> Vec<StageConstraint> {
        vec![
            StageConstraint::After("client_ip_from_header".to_string()),
            StageConstraint::Before("forward_proxy".to_string()),
            StageConstraint::Before("reverse_proxy".to_string()),
        ]
    }

    fn is_applicable(
        &self,
        config: Option<&ferron_core::config::ServerConfigurationBlock>,
    ) -> bool {
        config.is_some_and(|c| c.has_directive("basic_auth"))
    }

    #[inline]
    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        let config = match parse_basicauth_config(&ctx.configuration) {
            Some(cfg) => cfg,
            None => return Ok(true), // No basicauth configured — pass through
        };

        let engine = if let Some(engine) = self.engines.get(&config.brute_force) {
            // Fast path
            engine
        } else {
            // Use .entry(..).or_insert_with(..) to avoid insertion race conditions
            self.engines
                .entry(config.brute_force.clone())
                .or_insert_with(|| BruteForceEngine::new(config.brute_force.clone()))
                .downgrade()
        };

        // Extract the Authorization header
        let auth_header = ctx
            .req
            .as_ref()
            .and_then(|req| req.headers().get(http::header::AUTHORIZATION))
            .and_then(|v| v.to_str().ok());

        let auth_header = match auth_header {
            Some(h) => h,
            None => {
                // No credentials provided
                ctx.res = Some(Self::make_auth_challenge_response(ctx, &config.realm));
                return Ok(false);
            }
        };

        // Parse the Basic auth credentials
        let (username, password) = match Self::parse_basic_auth_header(auth_header) {
            Some(parts) => parts,
            None => {
                // Malformed credentials
                ctx.res = Some(Self::make_auth_challenge_response(ctx, &config.realm));
                return Ok(false);
            }
        };
        let ip = ctx.remote_address.ip();

        // Check brute-force lockout
        if engine.is_locked(ip) {
            ctx.events.emit(Event::Log(LogEvent {
                level: LogLevel::Warn,
                message: format!("basicauth: IP '{ip}' locked (brute-force protection)"),
                target: "ferron-http-basicauth",
                trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
            }));
            ctx.res = Some(Self::make_lockout_response(ctx, &engine));
            return Ok(false);
        }

        // Look up the user and verify the password
        let stored_hash = match config.users.get(&username) {
            Some(hash) => hash,
            None => {
                // Verify a fake hash to thwart user enumeration
                let _ = Self::verify_password("test", FAKE_HASH).await;

                // Unknown user — record failure for brute-force tracking
                engine.record_failure(ip);
                if let Some(recorder) = get_global_abuse_recorder() {
                    let abuse_event = AbuseEvent::new(
                        AbuseEventType::BruteForceFailure,
                        ip,
                        format!("Brute-force failure for unknown user '{}'", username),
                        60,
                    );
                    recorder.record_event(&abuse_event, ctx);
                }
                ctx.events.emit(Event::Log(LogEvent {
                    level: LogLevel::Warn,
                    message: format!(
                        "basicauth: authentication failed for unknown user '{}'",
                        username
                    ),
                    target: "ferron-http-basicauth",
                    trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
                }));
                ctx.res = Some(Self::make_auth_challenge_response(ctx, &config.realm));
                return Ok(false);
            }
        };

        if Self::verify_password(&password, stored_hash).await {
            // Authentication successful
            ctx.events.emit(Event::Log(LogEvent {
                level: LogLevel::Debug,
                message: format!("basicauth: user '{}' authenticated successfully", username),
                target: "ferron-http-basicauth",
                trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
            }));
            ctx.auth_user = Some(username);
            Ok(true) // Continue pipeline
        } else {
            // Authentication failed — record failure
            let locked = engine.record_failure(ctx.remote_address.ip());
            if let Some(recorder) = get_global_abuse_recorder() {
                let abuse_event = AbuseEvent::new(
                    AbuseEventType::BruteForceFailure,
                    ctx.remote_address.ip(),
                    format!("Brute-force failure for user '{}'", username),
                    60,
                );
                recorder.record_event(&abuse_event, ctx);
            }
            ctx.events.emit(Event::Log(LogEvent {
                level: LogLevel::Warn,
                message: format!(
                    "basicauth: authentication failed for user '{}'{}",
                    username,
                    if locked {
                        format!(" (client {ip} now locked)")
                    } else {
                        "".to_string()
                    }
                ),
                target: "ferron-http-basicauth",
                trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
            }));
            ctx.res = Some(Self::make_auth_challenge_response(ctx, &config.realm));
            Ok(false)
        }
    }
}

impl Default for BasicAuthStage {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use bytes::Bytes;
    use ferron_core::config::layer::LayeredConfiguration;
    use ferron_core::config::{
        ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue,
    };
    use ferron_http::HttpRequest;
    use ferron_observability::CompositeEventSink;
    use http::Request;
    use http_body_util::{BodyExt, Empty};
    use rustc_hash::FxHashMap;
    use std::collections::HashMap as StdHashMap;
    use std::sync::Arc;
    use typemap_rev::TypeMap;

    fn make_test_context_with_auth_header(
        auth_header: Option<&str>,
        config: Option<LayeredConfiguration>,
    ) -> HttpContext {
        let mut builder = Request::builder().uri("/path");
        if let Some(auth) = auth_header {
            builder = builder.header(http::header::AUTHORIZATION, auth);
        }
        let req: HttpRequest = builder
            .body(Empty::<Bytes>::new().map_err(|e| match e {}).boxed_unsync())
            .unwrap();

        HttpContext {
            req: Some(req),
            res: None,
            events: CompositeEventSink::new(Vec::new()),
            configuration: config.unwrap_or_default(),
            hostname: None,
            variables: FxHashMap::default(),
            previous_error: None,
            original_uri: None,
            routing_uri: None,
            encrypted: false,
            local_address: "0.0.0.0:80".parse().unwrap(),
            remote_address: "192.0.2.1:12345".parse().unwrap(),
            auth_user: None,
            https_port: None,
            extensions: TypeMap::new(),
        }
    }

    fn make_basicauth_config(users: Vec<(&str, &str)>) -> LayeredConfiguration {
        let mut inner_directives = StdHashMap::new();

        let mut users_block_directives = StdHashMap::new();
        for (username, hash) in users {
            users_block_directives.insert(
                username.to_string(),
                vec![ServerConfigurationDirectiveEntry {
                    args: vec![ServerConfigurationValue::String(hash.to_string(), None)],
                    children: None,
                    span: None,
                }],
            );
        }

        inner_directives.insert(
            "users".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![],
                children: Some(ServerConfigurationBlock {
                    directives: Arc::new(users_block_directives),
                    matchers: StdHashMap::new(),
                    span: None,
                }),
                span: None,
            }],
        );

        let mut directives = StdHashMap::new();
        directives.insert(
            "basic_auth".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![],
                children: Some(ServerConfigurationBlock {
                    directives: Arc::new(inner_directives),
                    matchers: StdHashMap::new(),
                    span: None,
                }),
                span: None,
            }],
        );

        let mut config = LayeredConfiguration::new();
        config.layers.push(Arc::new(ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: StdHashMap::new(),
            span: None,
        }));
        config
    }

    fn make_basic_auth_header(username: &str, password: &str) -> String {
        let credentials = format!("{username}:{password}");
        let encoded = base64::engine::general_purpose::STANDARD.encode(credentials.as_bytes());
        format!("Basic {encoded}")
    }

    #[tokio::test]
    async fn rejects_missing_auth_header() {
        let stage = BasicAuthStage::new();
        let config = make_basicauth_config(vec![("alice", "$argon2id$v=19$m=19456,t=2,p=1$abc")]);

        let mut ctx = make_test_context_with_auth_header(None, Some(config));
        let result = stage.run(&mut ctx).await.unwrap();

        assert!(!result, "should stop pipeline");
        assert!(ctx.res.is_some());
    }

    #[test]
    fn rejects_unknown_user() {
        // Had to use `vibeio`, since this test uses vibeio::spawn_blocking,
        // which would fail on Tokio.
        let rt = vibeio::RuntimeBuilder::new()
            .driver(vibeio::DriverKind::Mock)
            .default_blocking_pool(64)
            .build()
            .expect("failed to build runtime");
        rt.block_on(async move {
            let stage = BasicAuthStage::new();
            let config =
                make_basicauth_config(vec![("alice", "$argon2id$v=19$m=19456,t=2,p=1$abc")]);

            let auth_header = make_basic_auth_header("bob", "somepassword");
            let mut ctx = make_test_context_with_auth_header(Some(&auth_header), Some(config));
            let result = stage.run(&mut ctx).await.unwrap();

            assert!(!result, "should stop pipeline");
            assert!(ctx.res.is_some());
        });
    }

    #[tokio::test]
    async fn no_config_passes_through() {
        let stage = BasicAuthStage::new();

        let mut ctx = make_test_context_with_auth_header(None, None);
        let result = stage.run(&mut ctx).await.unwrap();

        assert!(result, "should continue pipeline");
        assert!(ctx.res.is_none());
    }

    #[test]
    fn parses_basic_auth_header_correctly() {
        let header = make_basic_auth_header("alice", "secret123");
        let (user, pass) = BasicAuthStage::parse_basic_auth_header(&header).unwrap();
        assert_eq!(user, "alice");
        assert_eq!(pass, "secret123");
    }

    #[test]
    fn rejects_malformed_auth_header() {
        assert!(BasicAuthStage::parse_basic_auth_header("InvalidFormat").is_none());
        assert!(BasicAuthStage::parse_basic_auth_header("Basic not-base64!!!").is_none());
        assert!(BasicAuthStage::parse_basic_auth_header("Basic bm9jb2xvbg==").is_none());
    }
}
