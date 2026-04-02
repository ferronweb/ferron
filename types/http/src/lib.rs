//! HTTP context types

pub mod variables;

use ferron_core::config::layer::LayeredConfiguration;
use ferron_core::config::Variables;
use ferron_observability::CompositeEventSink;
use http::{HeaderMap, Request, Response};
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
