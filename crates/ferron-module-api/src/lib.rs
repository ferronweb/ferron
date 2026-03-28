use std::any::Any;
use std::future::Future;
use std::pin::Pin;

pub trait Module: Send + Sync {
    fn name(&self) -> &str;
    fn as_any(&self) -> &dyn Any;
    fn start(&self) -> Pin<Box<dyn Future<Output = ()> + Send>>;
}
