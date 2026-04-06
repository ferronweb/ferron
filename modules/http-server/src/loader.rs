//! Module loader implementation

use std::collections::HashMap;
use std::sync::Arc;

use ferron_core::config::ServerConfigurationPort;
use ferron_core::loader::ModuleLoader;
use ferron_core::registry::RegistryBuilder;
use ferron_http::HttpContext;

use crate::server::BasicHttpModule;
use crate::stages::ClientIpFromHeaderStage;
use crate::stages::HttpsRedirectStage;
use crate::validator::HttpConfigurationValidator;

/// Default HTTP port when not explicitly configured.
const DEFAULT_HTTP_PORT: u16 = 80;
/// Default HTTPS port when not explicitly configured.
const DEFAULT_HTTPS_PORT: u16 = 443;

/// Returns true if the hostname is a loopback / development name that should
/// never get automatic TLS.
fn is_localhost_host(hostname: &str) -> bool {
    matches!(hostname, "localhost" | "127.0.0.1" | "::1")
}

/// Resolve the default HTTP port from global configuration.
fn resolve_default_http_port(config: &ferron_core::config::ServerConfiguration) -> u16 {
    config
        .global_config
        .directives
        .get("default_http_port")
        .and_then(|entries| entries.first())
        .and_then(|entry| entry.args.first())
        .and_then(|v| v.as_number())
        .and_then(|n| u16::try_from(n).ok())
        .unwrap_or(DEFAULT_HTTP_PORT)
}

/// Resolve the default HTTPS port from global configuration.
fn resolve_default_https_port(config: &ferron_core::config::ServerConfiguration) -> u16 {
    config
        .global_config
        .directives
        .get("default_https_port")
        .and_then(|entries| entries.first())
        .and_then(|entry| entry.args.first())
        .and_then(|v| v.as_number())
        .and_then(|n| u16::try_from(n).ok())
        .unwrap_or(DEFAULT_HTTPS_PORT)
}

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
        let default_port = resolve_default_http_port(config);
        let mut blocks = Vec::new();
        if let Some(ports) = config.ports.get("http") {
            for port in ports {
                for (filters, host) in &port.hosts {
                    // Build descriptive block name based on filters
                    let block_name = match (&filters.host, &filters.ip) {
                        (Some(hostname), Some(ip)) => {
                            format!(
                                "port {} host {} ip {}",
                                port.port.unwrap_or(default_port),
                                hostname,
                                ip
                            )
                        }
                        (Some(hostname), None) => {
                            format!(
                                "port {} host {}",
                                port.port.unwrap_or(default_port),
                                hostname
                            )
                        }
                        (None, Some(ip)) => {
                            format!("port {} ip {}", port.port.unwrap_or(default_port), ip)
                        }
                        (None, None) => {
                            format!("port {}", port.port.unwrap_or(default_port))
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
            .with_stage::<HttpContext, _>(|| Arc::new(ClientIpFromHeaderStage))
            .with_stage::<HttpContext, _>(|| Arc::new(HttpsRedirectStage))
    }

    fn register_modules(
        &mut self,
        registry: Arc<ferron_core::registry::Registry>,
        modules: &mut Vec<Arc<dyn ferron_core::Module>>,
        config: Arc<ferron_core::config::ServerConfiguration>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut new_cache = HashMap::new();
        if let Some(port_configs) = config.ports.get("http").cloned() {
            let mut port_configs_new: Vec<ServerConfigurationPort> = Vec::new();

            let default_port = resolve_default_http_port(&config);
            let default_https = resolve_default_https_port(&config);

            // Expand port configs: when no port is specified, create both HTTP and HTTPS
            // entries.  Localhost-like hostnames are excluded from the HTTPS listener.
            let mut expanded: Vec<ServerConfigurationPort> = Vec::new();
            for port_config in &port_configs {
                if port_config.port.is_some() {
                    // Explicit port — use as-is (no automatic TLS expansion)
                    expanded.push(port_config.clone());
                } else {
                    // Split hosts: localhost hosts go only to HTTP, others go to both.
                    let mut http_hosts = Vec::new();
                    let mut https_hosts = Vec::new();

                    for (filters, block) in &port_config.hosts {
                        let is_localhost = filters.host.as_deref().is_some_and(is_localhost_host);
                        http_hosts.push((filters.clone(), block.clone()));
                        if !is_localhost || block.directives.contains_key("tls") {
                            https_hosts.push((filters.clone(), block.clone()));
                        }
                    }

                    // HTTP listener gets all hosts (including localhost)
                    if !http_hosts.is_empty() {
                        let mut http_config = port_config.clone();
                        http_config.port = Some(default_port);
                        http_config.hosts = http_hosts;
                        expanded.push(http_config);
                    }

                    // HTTPS listener only gets non-localhost hosts
                    if !https_hosts.is_empty() {
                        let mut https_config = port_config.clone();
                        https_config.port = Some(default_https);
                        https_config.hosts = https_hosts;
                        expanded.push(https_config);
                    }
                }
            }

            // Merge port configurations with the same port number.
            for mut port_config in expanded {
                let port = port_config
                    .port
                    .expect("port should be set after expansion");
                if let Some(existing) = port_configs_new.iter_mut().find(|c| c.port == Some(port)) {
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
                let port = port_config.port.expect("invalid HTTP server module state");
                // https_port is used by the redirect stage to construct the target URL.
                // For implicit port configs (expanded to separate HTTP/HTTPS listeners),
                // use the HTTPS default so redirects target the HTTPS listener.
                // For explicit port configs (single listener), set it equal to the listener
                // port so the redirect stage skips (no separate HTTPS listener exists).
                let is_explicit_port = port_configs.iter().any(|pc| pc.port == Some(port));
                let https_port = if is_explicit_port {
                    port // Same port → redirect stage will skip
                } else {
                    default_https
                };

                if let Some(cached) = self.cache.get(&port) {
                    // Configuration reload: update the cached module with new configuration
                    cached.reload(
                        &registry,
                        port_config,
                        config.global_config.clone(),
                        https_port,
                    )?;
                    new_cache.insert(port, cached.clone());
                } else {
                    let http_module = Arc::new(BasicHttpModule::new(
                        &registry,
                        port_config,
                        config.global_config.clone(),
                        port,
                        https_port,
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
