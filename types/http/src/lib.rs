//! HTTP context types

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
}
