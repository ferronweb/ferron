use std::any::Any;
use std::future::Future;
use std::pin::Pin;

use ferron_core::http::HttpContext;
use ferron_runtime::pipeline::Pipeline;

pub trait Module: Send + Sync {
    fn name(&self) -> &str;
    fn as_any(&self) -> &dyn Any;
}

// Capability trait

pub trait HttpModule: Send + Sync {
    fn register(&self, pipeline: Pipeline<HttpContext>) -> Pipeline<HttpContext>;
}

pub trait ServerModule: Send + Sync {
    fn start(&self) -> Pin<Box<dyn Future<Output = ()> + Send>>;
}

// Bridge trait

pub trait ProvidesHttp {
    fn http(&self) -> Option<&dyn HttpModule> {
        None
    }
}

pub trait ProvidesServer {
    fn server(&self) -> Option<&dyn ServerModule> {
        None
    }
}

pub trait FerronModule: Module + ProvidesServer + ProvidesHttp {}

impl<T> FerronModule for T where T: Module + ProvidesServer + ProvidesHttp {}
