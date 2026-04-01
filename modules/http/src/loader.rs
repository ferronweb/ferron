//! Module loader implementation

use std::collections::HashMap;
use std::sync::Arc;

use ferron_core::loader::ModuleLoader;
use ferron_core::registry::RegistryBuilder;

use crate::context::HttpContext;
use crate::server::BasicHttpModule;
use crate::stages::{HelloStage, LoggingStage, NotFoundStage};

#[derive(Default)]
pub struct BasicHttpModuleLoader {
    cache: HashMap<u16, Arc<BasicHttpModule>>,
}

impl ModuleLoader for BasicHttpModuleLoader {
    fn register_per_protocol_configuration_blocks<'a>(
        &mut self,
        config: &'a ferron_core::config::ServerConfiguration,
        registry: &mut HashMap<
            &'static str,
            Vec<(String, &'a ferron_core::config::ServerConfigurationBlock)>,
        >,
    ) {
        let mut blocks = Vec::new();
        if let Some(ports) = config.ports.get("http") {
            for port in ports {
                for (_filters, host) in &port.hosts {
                    // TODO: more sophisticated block naming based on filters
                    blocks.push((format!("port {}", port.port.unwrap_or(80)), host));
                }
            }
        }
        registry.insert("http", blocks);
    }

    // TODO: configuration validators

    fn register_stages(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        registry
            .with_stage::<HttpContext, _>(|| Arc::new(LoggingStage::default()))
            .with_stage::<HttpContext, _>(|| Arc::new(HelloStage::default()))
            .with_stage::<HttpContext, _>(|| Arc::new(NotFoundStage::default()))
    }

    fn register_modules(
        &mut self,
        registry: &ferron_core::registry::Registry,
        modules: &mut Vec<Arc<dyn ferron_core::Module>>,
        config: &mut ferron_core::config::ServerConfiguration,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut new_cache = HashMap::new();
        if let Some(port_config) = config.ports.remove("http") {
            for port_config in port_config {
                // TODO: automatic TLS by default
                //       also, determine whether to enable port 443, for example when TLS is enabled by default
                let port = port_config.port.unwrap_or(80);
                if let Some(cached) = self.cache.get(&port) {
                    new_cache.insert(port, cached.clone());
                } else {
                    let http_module = Arc::new(BasicHttpModule::new(
                        registry,
                        port_config,
                        config.global_config.clone(),
                        port,
                    )?);
                    modules.push(http_module.clone());
                    new_cache.insert(port, http_module);
                }
            }
        }
        self.cache = new_cache;
        Ok(())
    }
}
