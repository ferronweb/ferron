//! HTTP context types for Ferron modules.
//!
//! This crate defines the core types used throughout the HTTP processing
//! pipeline. Modules interact with request and response data through
//! [`HttpContext`], which carries the request, configuration, observability
//! sink, and per-request extensions.
//!
//! # Key types
//!
//! - [`HttpContext`] — the per-request context passed through HTTP stages.
//! - [`HttpRequest`] — type alias for `http::Request<UnsyncBoxBody<Bytes, io::Error>>`.
//! - [`HttpResponse`] — an enum that represents either a custom response, a builtin error, or an abort.
//! - [`HttpFileContext`] — extends `HttpContext` with file-serving state.
//! - [`HttpErrorContext`] — context for error page rendering.

#[cfg(feature = "abuse")]
pub mod abuse;
pub mod access_log;
pub mod client_ip;
pub mod file_descriptor;
#[cfg(feature = "mtls")]
pub mod mtls;
pub mod span;
pub mod trace_context;
#[cfg(feature = "util")]
pub mod util;
pub mod variables;

use std::net::SocketAddr;

use ferron_core::config::layer::LayeredConfiguration;
use ferron_core::config::Variables;
use ferron_observability::CompositeEventSink;
use http::{HeaderMap, Request, Response, Uri};
use http_body_util::combinators::UnsyncBoxBody;
use rustc_hash::FxHashMap;
use typemap_rev::{TypeMap, TypeMapKey};

use crate::file_descriptor::ReusedFile;

/// An HTTP request with an unsync-boxed body.
///
/// This is the standard request type used throughout the HTTP pipeline.
pub type HttpRequest = Request<UnsyncBoxBody<bytes::Bytes, std::io::Error>>;

/// An HTTP response or signal to use a builtin error page.
///
/// The HTTP server stage converts this into an actual `http::Response` before
/// sending it to the client.
pub enum HttpResponse {
    /// A fully formed response with a body.
    Custom(Response<UnsyncBoxBody<bytes::Bytes, std::io::Error>>),
    /// Use the builtin error page for the given status code, with optional header overrides.
    BuiltinError(u16, Option<HeaderMap>),
    /// Abort the connection without sending a response.
    Abort,
}

/// Per-request HTTP processing context.
///
/// `HttpContext` is passed through every HTTP processing stage. It carries
/// the request, response, configuration, observability sink, and extensible
/// per-request state via a type map.
///
/// Modules store and retrieve per-request state through the [`extensions`](HttpContext::extensions)
/// type map. For example, the trace context module stores a [`TraceContextKey`](trace_context::TraceContextKey).
#[derive(Default)]
#[non_exhaustive]
pub struct HttpContext {
    /// The incoming HTTP request, if available.
    pub req: Option<HttpRequest>,
    /// The outgoing HTTP response, set by a processing stage.
    pub res: Option<HttpResponse>,
    /// The observability event sink for this request.
    pub events: CompositeEventSink,
    /// Layered configuration (global + host + location).
    pub configuration: LayeredConfiguration,
    /// The matched hostname, if any.
    pub hostname: Option<String>,
    /// Custom variables set by modules (accessible via `resolve_variable`).
    pub variables: FxHashMap<String, String>,
    /// The HTTP status code from a previous error page, if any.
    pub previous_error: Option<u16>,
    /// The original request URI before any rewriting.
    pub original_uri: Option<Uri>,
    /// The URI used for routing after rewriting.
    pub routing_uri: Option<Uri>,
    /// Whether the connection is encrypted (TLS).
    pub encrypted: bool,
    /// The local socket address the client connected to.
    pub local_address: Option<SocketAddr>,
    /// The remote client socket address.
    pub remote_address: Option<SocketAddr>,
    /// The authenticated username, if any.
    pub auth_user: Option<String>,
    /// The port used for HTTPS redirection (e.g. `Some(443)`).
    pub https_port: Option<u16>,
    /// Whether to suppress the `Server` header in responses.
    pub hide_server: bool,
    /// Extensible per-request state. Modules store typed values here.
    pub extensions: TypeMap,
}

impl Variables for HttpContext {
    fn resolve(&self, key: &str) -> Option<String> {
        variables::resolve_variable(key, self)
    }
}

impl HttpContext {
    /// Insert a value into the extensions type map.
    ///
    /// If a value of this type already exists, it will be replaced.
    pub fn insert<T: TypeMapKey>(&mut self, value: T::Value) {
        self.extensions.insert::<T>(value);
    }

    /// Get a reference to a value from the extensions type map.
    pub fn get<T: TypeMapKey>(&self) -> Option<&T::Value> {
        self.extensions.get::<T>()
    }

    /// Get a mutable reference to a value from the extensions type map.
    pub fn get_mut<T: TypeMapKey>(&mut self) -> Option<&mut T::Value> {
        self.extensions.get_mut::<T>()
    }

    /// Remove a value from the extensions type map and return it.
    pub fn remove<T: TypeMapKey>(&mut self) -> Option<T::Value> {
        self.extensions.remove::<T>()
    }

    /// Check if a value of the given type exists in the extensions type map.
    pub fn contains<T: TypeMapKey>(&self) -> bool {
        self.extensions.contains_key::<T>()
    }
}

/// Per-request context for static file serving.
///
/// Extends [`HttpContext`] with file-specific state resolved during the
/// file-serving stage (path, root, ETag, and the open file handle).
#[derive(Default)]
#[non_exhaustive]
pub struct HttpFileContext {
    /// The base HTTP context.
    pub http: HttpContext,
    /// The resolved absolute file path on disk.
    pub file_path: std::path::PathBuf,
    /// Path info appended after the file path (e.g. `/test` in `/index.php/test`).
    pub path_info: Option<String>,
    /// The document root directory for this file.
    pub file_root: std::path::PathBuf,
    /// Pre-computed ETag from the path resolve cache.
    pub etag: String,
    /// The open file handle, if the file was found.
    pub file: Option<ReusedFile>,
}

/// Per-request context for error page rendering.
///
/// Carries the error code, optional header overrides, and configuration
/// needed to render a custom error page.
#[derive(Default)]
#[non_exhaustive]
pub struct HttpErrorContext {
    /// The HTTP error status code (e.g. `404`, `500`).
    pub error_code: u16,
    /// Optional header overrides for the error response.
    pub headers: Option<HeaderMap>,
    /// The layered configuration for error page rules.
    pub configuration: LayeredConfiguration,
    /// Trace context for the request that triggered the error.
    pub trace_context: Option<crate::trace_context::TraceContext>,
    /// The rendered error response, if already built.
    pub res: Option<Response<UnsyncBoxBody<bytes::Bytes, std::io::Error>>>,
    /// Custom variables available for error page templates.
    pub variables: FxHashMap<String, String>,
}

impl Variables for HttpErrorContext {
    fn resolve(&self, name: &str) -> Option<String> {
        self.variables
            .get(name)
            .cloned()
            .or_else(|| Some(name.to_string()))
    }
}
