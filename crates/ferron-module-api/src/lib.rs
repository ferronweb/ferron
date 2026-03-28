use std::any::Any;
use std::future::Future;
use std::pin::Pin;

pub trait Module: Send + Sync {
    fn name(&self) -> &str;
    fn as_any(&self) -> &dyn Any;
}

// Capability trait

pub trait ServerModule: Send + Sync {
    fn start(&self) -> Pin<Box<dyn Future<Output = ()> + Send>>;
}

// Bridge trait

pub trait ProvidesServer {
    fn server(&self) -> Option<&dyn ServerModule> {
        None
    }
}

pub trait FerronModule: Module + ProvidesServer {}

impl<T> FerronModule for T where T: Module + ProvidesServer {}
