//! Module loader implementation

use std::collections::HashMap;
use std::sync::Arc;

use ferron_common::loader::ModuleLoader;
use ferron_common::registry::Registry;
use ferron_common::registry::RegistryBuilder;
use ferron_common::Module;

use crate::context::HttpContext;
use crate::server::BasicHttpModule;
use crate::stages::{HelloStage, LoggingStage, NotFoundStage};

// TODO: "cache" modules in the loader for graceful reloads
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
        if let Some(port_config) = config.ports.remove("http") {
            for port_config in port_config {
                if let Some(_cached) = self.cache.get(&port_config.port) {
                    // TODO: reload configuration in existing HTTP server
                } else {
                    let port = port_config.port;
                    let http_module = Arc::new(BasicHttpModule::new(
                        registry,
                        port_config,
                        config.global_config.clone(),
                    ));
                    modules.push(http_module.clone());
                    self.cache.insert(port, http_module);
                }
            }
        }
    }
}
