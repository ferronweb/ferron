//! HTTP context types

pub mod variables;

use std::net::SocketAddr;

use ferron_core::config::layer::LayeredConfiguration;
use ferron_core::config::Variables;
use ferron_observability::CompositeEventSink;
use http::{HeaderMap, Request, Response, Uri};
use http_body_util::combinators::UnsyncBoxBody;

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
    pub variables: std::collections::HashMap<String, String>,
    pub previous_error: Option<u16>,
    pub original_uri: Option<Uri>,
    pub encrypted: bool,
    pub local_address: SocketAddr,
    pub remote_address: SocketAddr,
}

impl Variables for HttpContext {
    fn resolve(&self, key: &str) -> Option<String> {
        if let Some(req) = &self.req {
            variables::resolve_variable(key, req, &self.variables)
        } else {
            self.variables.resolve(key)
        }
    }
}

pub struct HttpFileContext {
    pub http: HttpContext,
    pub metadata: vibeio::fs::Metadata,
    pub file_path: std::path::PathBuf,
    pub path_info: Option<String>, // For example, "/test" in "/index.php/test"
    pub file_root: std::path::PathBuf,
}
