//! HTTP context types

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

pub type HttpRequest = Request<UnsyncBoxBody<bytes::Bytes, std::io::Error>>;
pub enum HttpResponse {
    Custom(Response<UnsyncBoxBody<bytes::Bytes, std::io::Error>>),
    BuiltinError(u16, Option<HeaderMap>),
    Abort,
}

pub struct HttpContext {
    pub req: Option<HttpRequest>,
    pub res: Option<HttpResponse>,
    pub events: CompositeEventSink,
    pub configuration: LayeredConfiguration,
    pub hostname: Option<String>,
    pub variables: FxHashMap<String, String>,
    pub previous_error: Option<u16>,
    pub original_uri: Option<Uri>,
    pub routing_uri: Option<Uri>,
    pub encrypted: bool,
    pub local_address: SocketAddr,
    pub remote_address: SocketAddr,
    pub auth_user: Option<String>,
    // For example, Some(443) for encrypted port 443
    // or Some(443) for implicit default HTTP non-encrypted port
    pub https_port: Option<u16>,
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

pub struct HttpFileContext {
    pub http: HttpContext,
    pub metadata: vibeio::fs::Metadata,
    pub file_path: std::path::PathBuf,
    pub path_info: Option<String>, // For example, "/test" in "/index.php/test"
    pub file_root: std::path::PathBuf,
    /// Pre-computed ETag from the path resolve cache.
    pub etag: String,
}

pub struct HttpErrorContext {
    pub error_code: u16,
    pub headers: Option<HeaderMap>,
    pub configuration: LayeredConfiguration,
    pub res: Option<Response<UnsyncBoxBody<bytes::Bytes, std::io::Error>>>,
}
