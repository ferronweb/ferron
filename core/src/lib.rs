#[macro_use]
pub mod config;
pub mod builtin;
pub mod loader;
pub mod logging;
pub mod pipeline;
pub mod providers;
pub mod registry;
pub mod runtime;
pub mod shutdown;

pub use registry::StageConstraint;

use std::any::Any;

pub trait Module: Send + Sync {
    fn name(&self) -> &str;
    fn as_any(&self) -> &dyn Any;
    fn start(
        &self,
        runtime: &mut crate::runtime::Runtime,
    ) -> Result<(), Box<dyn std::error::Error>>;
}
