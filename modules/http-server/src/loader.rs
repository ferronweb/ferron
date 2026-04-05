//! Module loader implementation

use std::collections::HashMap;
use std::sync::Arc;

use ferron_core::loader::ModuleLoader;
use ferron_core::registry::RegistryBuilder;
use ferron_http::{HttpContext, HttpErrorContext};

use crate::server::BasicHttpModule;
use crate::stages::{HelloStage, NotFoundStage};
use crate::validator::HttpConfigurationValidator;

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
                for (filters, host) in &port.hosts {
                    // Build descriptive block name based on filters
                    let block_name = match (&filters.host, &filters.ip) {
                        (Some(hostname), Some(ip)) => {
                            format!(
                                "port {} host {} ip {}",
                                port.port.unwrap_or(80),
                                hostname,
                                ip
                            )
                        }
                        (Some(hostname), None) => {
                            format!("port {} host {}", port.port.unwrap_or(80), hostname)
                        }
                        (None, Some(ip)) => {
                            format!("port {} ip {}", port.port.unwrap_or(80), ip)
                        }
                        (None, None) => {
                            format!("port {}", port.port.unwrap_or(80))
                        }
                    };
                    blocks.push((block_name, host));
                }
            }
        }
        registry.insert("http", blocks);
    }

    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn ferron_core::config::validator::ConfigurationValidator>>,
    ) {
        registry.push(Box::new(HttpConfigurationValidator));
    }

    fn register_stages(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        registry
            .with_stage::<HttpContext, _>(|| Arc::new(HelloStage))
            .with_stage::<HttpErrorContext, _>(|| Arc::new(NotFoundStage))
    }

    fn register_modules(
        &mut self,
        registry: Arc<ferron_core::registry::Registry>,
        modules: &mut Vec<Arc<dyn ferron_core::Module>>,
        config: Arc<ferron_core::config::ServerConfiguration>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut new_cache = HashMap::new();
        if let Some(port_configs) = config.ports.get("http").cloned() {
            let mut port_configs_new: Vec<ferron_core::config::ServerConfigurationPort> =
                Vec::new();

            // First merge port configurations with the same port number,
            // then check if we have a cached module for that port.
            // If we do, reuse it, otherwise create a new one.
            for mut port_config in port_configs {
                let port = port_config.port.unwrap_or(80);
                port_config.port = Some(port);
                if let Some(existing) = port_configs_new
                    .iter_mut()
                    .find(|c| c.port.unwrap_or(80) == port)
                {
                    // Merge hosts
                    let mut new_hosts = Vec::new();
                    for existing_host in existing.hosts.iter_mut() {
                        if let Some((_, new_block)) = port_config
                            .hosts
                            .iter_mut()
                            .find(|(filters, _)| filters == &existing_host.0)
                        {
                            // Merge the configuration blocks
                            let mut merged_block = HashMap::new();
                            merged_block.extend(
                                existing_host
                                    .1
                                    .directives
                                    .iter()
                                    .map(|(k, v)| (k.clone(), v.clone())),
                            );
                            merged_block.extend(
                                new_block
                                    .directives
                                    .iter()
                                    .map(|(k, v)| (k.clone(), v.clone())),
                            );
                            new_block.directives = Arc::new(merged_block);
                        } else {
                            new_hosts.push(existing_host.clone());
                        }
                    }
                    existing.hosts.extend(new_hosts);
                } else {
                    port_configs_new.push(port_config);
                }
            }

            for port_config in port_configs_new {
                // TODO: automatic TLS by default
                //       also, determine whether to enable port 443, for example when TLS is enabled by default
                let port = port_config.port.expect("invalid HTTP server module state");

                if let Some(cached) = self.cache.get(&port) {
                    // Configuration reload: update the cached module with new configuration
                    cached.reload(&registry, port_config, config.global_config.clone())?;
                    new_cache.insert(port, cached.clone());
                } else {
                    let http_module = Arc::new(BasicHttpModule::new(
                        &registry,
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
