//! Module loader implementation

use std::collections::HashMap;
use std::sync::Arc;

use ferron_common::loader::ModuleLoader;
use ferron_common::registry::RegistryBuilder;

use crate::context::HttpContext;
use crate::server::BasicHttpModule;
use crate::stages::{HelloStage, LoggingStage, NotFoundStage};

#[derive(Default)]
pub struct BasicHttpModuleLoader {
    cache: HashMap<u16, Arc<BasicHttpModule>>,
}

impl ModuleLoader for BasicHttpModuleLoader {
    fn register_stages(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        registry
            .with_stage::<HttpContext, _>(|| Arc::new(LoggingStage::default()))
            .with_stage::<HttpContext, _>(|| Arc::new(HelloStage::default()))
            .with_stage::<HttpContext, _>(|| Arc::new(NotFoundStage::default()))
    }

    fn register_modules(
        &mut self,
        registry: &ferron_common::registry::Registry,
        modules: &mut Vec<Arc<dyn ferron_common::Module>>,
        config: &mut ferron_common::config::ServerConfiguration,
    ) {
        let mut new_cache = HashMap::new();
        if let Some(port_config) = config.ports.remove("http") {
            for port_config in port_config {
                if let Some(cached) = self.cache.get(&port_config.port) {
                    // TODO: reload configuration in existing HTTP server
                    new_cache.insert(port_config.port, cached.clone());
                } else {
                    let port = port_config.port;
                    let http_module = Arc::new(BasicHttpModule::new(
                        registry,
                        port_config,
                        config.global_config.clone(),
                    ));
                    modules.push(http_module.clone());
                    new_cache.insert(port, http_module);
                }
            }
        }
        self.cache = new_cache;
    }
}
