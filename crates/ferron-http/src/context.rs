//! HTTP context types

use http::{Request, Response};

type HttpRequest = Request<Vec<u8>>;
type HttpResponse = Response<Vec<u8>>;

pub struct HttpContext {
    pub req: HttpRequest,
    pub res: HttpResponse,
}

impl HttpContext {
    pub fn new(req: HttpRequest) -> Self {
        Self {
            req,
            res: Response::builder()
                .status(200)
                .body(Vec::new())
                .expect("Failed to build default response"),
        }
    }
}
