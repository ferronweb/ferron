//! Forwarded authentication pipeline stage.

use std::sync::Arc;

use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::StageConstraint;
use ferron_http::HttpContext;
use ferron_observability::{Event, LogEvent, LogLevel};
use http::Request;
use http_body_util::{BodyExt, Empty};

use crate::client::ForwardedAuthClient;
use crate::config::{parse_forwarded_auth_from_context, ForwardedAuthConfig};
use crate::{ConnpoolKey, ProxyBody};

/// Pipeline stage that handles forwarded authentication.
pub struct ForwardedAuthenticationStage {
    /// Shared HTTP client for authentication requests.
    client: Arc<ForwardedAuthClient>,
}

impl ForwardedAuthenticationStage {
    /// Create a new forwarded authentication stage.
    pub fn new(client: Arc<ForwardedAuthClient>) -> Self {
        Self { client }
    }

    fn client_ip_from_header_enabled(ctx: &HttpContext) -> bool {
        ctx.configuration
            .get_value("client_ip_from_header", false)
            .and_then(|v| v.as_str())
            .is_some()
    }

    fn set_x_forwarded_for(headers: &mut http::HeaderMap, client_ip_str: &str) {
        if let Ok(hv) = http::HeaderValue::from_str(client_ip_str) {
            headers.insert("x-forwarded-for", hv);
        }
    }

    fn append_x_forwarded_for(headers: &mut http::HeaderMap, client_ip_str: &str) {
        if let Some(existing) = headers.get("x-forwarded-for") {
            if let Ok(existing_str) = existing.to_str() {
                let new_value = format!("{}, {}", existing_str, client_ip_str);
                if let Ok(hv) = http::HeaderValue::from_str(&new_value) {
                    headers.insert("x-forwarded-for", hv);
                    return;
                }
            }
        }
        if let Ok(hv) = http::HeaderValue::from_str(client_ip_str) {
            headers.insert("x-forwarded-for", hv);
        }
    }

    fn set_forwarded(
        headers: &mut http::HeaderMap,
        client_ip_str: &str,
        proto: &'static str,
        local_ip_str: &str,
    ) {
        let element = Self::build_forwarded_element(client_ip_str, proto, local_ip_str);
        if let Ok(hv) = http::HeaderValue::from_str(&element) {
            headers.insert("forwarded", hv);
        }
    }

    fn append_forwarded(
        headers: &mut http::HeaderMap,
        client_ip_str: &str,
        proto: &'static str,
        local_ip_str: &str,
    ) {
        let element = Self::build_forwarded_element(client_ip_str, proto, local_ip_str);
        if let Some(existing) = headers.get("forwarded") {
            if let Ok(existing_str) = existing.to_str() {
                let new_value = format!("{}, {}", existing_str, element);
                if let Ok(hv) = http::HeaderValue::from_str(&new_value) {
                    headers.insert("forwarded", hv);
                    return;
                }
            }
        }
        if let Ok(hv) = http::HeaderValue::from_str(&element) {
            headers.insert("forwarded", hv);
        }
    }

    fn build_forwarded_element(client_ip_str: &str, proto: &str, local_ip_str: &str) -> String {
        // Detect IPv6 from the pre-formatted string
        let for_value = if client_ip_str.contains(':') {
            format!("\"[{}]\"", client_ip_str)
        } else {
            client_ip_str.to_string()
        };
        let by_value = if local_ip_str.contains(':') {
            format!("\"[{}]\"", local_ip_str)
        } else {
            local_ip_str.to_string()
        };
        format!("for={};proto={};by={}", for_value, proto, by_value)
    }

    /// Build the authentication request to send to the backend.
    fn build_auth_request(
        &self,
        ctx: &HttpContext,
        config: &ForwardedAuthConfig,
    ) -> Result<Request<ProxyBody>, Box<dyn std::error::Error>> {
        let original_request = ctx.req.as_ref().ok_or("No request in context")?;

        // Build the URI for the auth request
        let path = original_request.uri().path();
        let query = original_request
            .uri()
            .query()
            .map(|q| format!("?{}", q))
            .unwrap_or_default();
        let auth_uri = format!("{}{}{}", config.backend_url, path, query);

        // Create the auth request
        let mut auth_request = Request::builder()
            .uri(auth_uri)
            .method(original_request.method().clone());

        // Copy headers from original request to auth request
        let headers = auth_request.headers_mut().unwrap();
        for (name, value) in original_request.headers() {
            headers.insert(name.clone(), value.clone());
        }

        // X-Forwarded headers
        let client_ip = ctx.remote_address.ip();
        let local_ip = ctx.local_address.ip();
        let proto = if ctx.encrypted { "https" } else { "http" };

        // Pre-format IPs once to avoid repeated allocations in header helpers
        let client_ip_str = client_ip.to_string();
        let local_ip_str = local_ip.to_string();

        // Add X-Forwarded-* headers
        if Self::client_ip_from_header_enabled(ctx) {
            Self::append_x_forwarded_for(headers, &client_ip_str);
            Self::append_forwarded(headers, &client_ip_str, proto, &local_ip_str);
        } else {
            Self::set_x_forwarded_for(headers, &client_ip_str);
            Self::set_forwarded(headers, &client_ip_str, proto, &local_ip_str);
        }
        headers.insert(
            http::header::HeaderName::from_static("x-forwarded-proto"),
            http::header::HeaderValue::from_static(proto),
        );
        headers.insert(
            http::header::HeaderName::from_static("x-forwarded-uri"),
            http::header::HeaderValue::from_str(&format!("{}{}", path, query))?,
        );
        headers.insert(
            http::header::HeaderName::from_static("x-forwarded-method"),
            http::header::HeaderValue::from_str(original_request.method().as_str())?,
        );

        // Remove upgrade headers (no HTTP upgrades for auth requests)
        headers.remove(http::header::UPGRADE);
        headers.remove(http::header::CONNECTION);
        headers.insert(
            http::header::CONNECTION,
            http::header::HeaderValue::from_static("keep-alive"),
        );

        Ok(auth_request.body(
            Empty::<bytes::Bytes>::new()
                .map_err(|e| match e {})
                .boxed_unsync(),
        )?)
    }

    /// Send the authentication request and handle the response.
    async fn send_auth_request(
        &self,
        ctx: &mut HttpContext,
        config: &ForwardedAuthConfig,
    ) -> Result<bool, PipelineError> {
        let auth_request = match self.build_auth_request(ctx, config) {
            Ok(req) => req,
            Err(e) => {
                ctx.events.emit(Event::Log(LogEvent {
                    level: LogLevel::Error,
                    message: format!("fauth: failed to build auth request: {}", e),
                    target: "ferron-http-fauth",
                }));
                ctx.res = Some(ferron_http::HttpResponse::BuiltinError(500, None));
                return Ok(false);
            }
        };

        // Create connection pool key
        let pool_key = ConnpoolKey {
            url: config.backend_url.clone(),
            unix_socket: config.unix_socket.clone(),
        };

        // Set and get local limit for the connection pool
        if let Some(limit) = config.connection_limit {
            self.client.set_local_limit(&pool_key.url, limit).await;
        }
        let local_limit = self.client.get_local_limit(&pool_key.url).await;

        // Get connection from pool
        let mut conn_item = match self
            .client
            .get_connection(&pool_key, config.no_verification, local_limit)
            .await
        {
            Ok(conn) => conn,
            Err(e) => {
                ctx.events.emit(Event::Log(LogEvent {
                    level: LogLevel::Error,
                    message: format!("fauth: failed to get connection: {}", e),
                    target: "ferron-http-fauth",
                }));
                ctx.res = Some(ferron_http::HttpResponse::BuiltinError(500, None));
                return Ok(false);
            }
        };

        // Send the authentication request
        let auth_response = match conn_item
            .inner_mut()
            .as_mut()
            .unwrap()
            .client
            .send_request(auth_request)
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                ctx.events.emit(Event::Log(LogEvent {
                    level: LogLevel::Error,
                    message: format!("fauth: auth request failed: {}", e),
                    target: "ferron-http-fauth",
                }));
                // Return connection to pool even on error
                self.client.return_connection(pool_key, conn_item);
                ctx.res = Some(ferron_http::HttpResponse::BuiltinError(500, None));
                return Ok(false);
            }
        };

        // Check if authentication was successful
        if auth_response.status().is_success() {
            // Authentication successful - copy headers if configured
            if !config.copy_headers.is_empty() {
                let original_request = ctx
                    .req
                    .as_mut()
                    .ok_or(PipelineError::custom("No request in context"))?;
                let auth_headers = auth_response.headers();
                let request_headers = original_request.headers_mut();

                for header_name in &config.copy_headers {
                    if auth_headers.contains_key(header_name) {
                        // Remove existing headers
                        while request_headers.remove(header_name).is_some() {}
                        // Copy all values from auth response
                        for header_value in auth_headers.get_all(header_name) {
                            request_headers.append(header_name.clone(), header_value.clone());
                        }
                    }
                }
            }

            // Return connection to pool
            self.client.return_connection(pool_key, conn_item);

            ctx.events.emit(Event::Log(LogEvent {
                level: LogLevel::Debug,
                message: "fauth: authentication successful".to_string(),
                target: "ferron-http-fauth",
            }));

            Ok(true) // Continue pipeline
        } else {
            let auth_status = auth_response.status();
            // Authentication failed - return the auth response as the final response
            ctx.res = Some(ferron_http::HttpResponse::Custom(auth_response.map(
                |body| {
                    // Convert body to a type that can be used in HttpResponse
                    body.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
                        .boxed_unsync()
                },
            )));

            // Return connection to pool
            self.client.return_connection(pool_key, conn_item);

            ctx.events.emit(Event::Log(LogEvent {
                level: LogLevel::Info,
                message: format!("fauth: authentication failed with status {}", auth_status),
                target: "ferron-http-fauth",
            }));

            Ok(false) // Stop pipeline
        }
    }
}

#[async_trait::async_trait(?Send)]
impl Stage<HttpContext> for ForwardedAuthenticationStage {
    fn name(&self) -> &str {
        "forwarded_auth"
    }

    fn constraints(&self) -> Vec<StageConstraint> {
        vec![
            StageConstraint::Before("reverse_proxy".to_string()),
            StageConstraint::Before("forward_proxy".to_string()),
            StageConstraint::After("cache".to_string()),
            StageConstraint::After("basicauth".to_string()),
        ]
    }

    fn is_applicable(
        &self,
        config: Option<&ferron_core::config::ServerConfigurationBlock>,
    ) -> bool {
        config.is_some_and(|c| c.has_directive("auth_to"))
    }

    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        let config = match parse_forwarded_auth_from_context(ctx) {
            Ok(Some(cfg)) => cfg,
            Ok(None) => return Ok(true), // No forwarded auth configured
            Err(e) => {
                ctx.events.emit(Event::Log(LogEvent {
                    level: LogLevel::Error,
                    message: format!("fauth: configuration error: {}", e),
                    target: "ferron-http-fauth",
                }));
                return Ok(true); // Continue pipeline on config error
            }
        };

        self.send_auth_request(ctx, &config).await
    }
}
