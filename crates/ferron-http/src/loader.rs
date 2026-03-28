//! Module loader implementation

use std::sync::Arc;

use ferron_common::loader::ModuleLoader;
use ferron_common::registry::Registry;
use ferron_common::registry::RegistryBuilder;
use ferron_common::Module;

use crate::context::HttpContext;
use crate::server::BasicHttpModule;
use crate::stages::{HelloStage, LoggingStage, NotFoundStage};

pub struct BasicHttpModuleLoader;

impl ModuleLoader for BasicHttpModuleLoader {
    fn register_stages(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        registry
            .with_stage::<HttpContext, _>(|| Arc::new(LoggingStage::default()))
            .with_stage::<HttpContext, _>(|| Arc::new(HelloStage::default()))
            .with_stage::<HttpContext, _>(|| Arc::new(NotFoundStage::default()))
    }

    fn register_modules(&mut self, registry: &Registry, modules: &mut Vec<Box<dyn Module>>) {
        let http_module = BasicHttpModule::from_registry(&registry);
        modules.push(Box::new(http_module));
    }
}
