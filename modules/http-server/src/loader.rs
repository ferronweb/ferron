//! Module loader implementation

use std::collections::HashMap;
use std::sync::Arc;

use ferron_core::builtin::BuiltinConfigurationValidator;
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

/// Resolve the default HTTP port from global configuration.
/// Returns `None` if `default_http_port false` is set.
fn resolve_default_http_port(config: &ferron_core::config::ServerConfiguration) -> Option<u16> {
    match config
        .global_config
        .directives
        .get("default_http_port")
        .and_then(|entries| entries.first())
        .and_then(|entry| entry.args.first())
    {
        Some(v) => {
            if let Some(b) = v.as_boolean() {
                // `false` means disabled, `true` would be odd but use default
                if b {
                    Some(DEFAULT_HTTP_PORT)
                } else {
                    None
                }
            } else {
                v.as_number().and_then(|n| u16::try_from(n).ok())
            }
        }
        None => Some(DEFAULT_HTTP_PORT),
    }
}

/// Resolve the default HTTPS port from global configuration.
/// Returns `None` if `default_https_port false` is set.
fn resolve_default_https_port(config: &ferron_core::config::ServerConfiguration) -> Option<u16> {
    match config
        .global_config
        .directives
        .get("default_https_port")
        .and_then(|entries| entries.first())
        .and_then(|entry| entry.args.first())
    {
        Some(v) => {
            if let Some(b) = v.as_boolean() {
                if b {
                    Some(DEFAULT_HTTPS_PORT)
                } else {
                    None
                }
            } else {
                v.as_number().and_then(|n| u16::try_from(n).ok())
            }
        }
        None => Some(DEFAULT_HTTPS_PORT),
    }
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
                // Skip host blocks that won't create any listeners
                let effective_port = port.port.or(default_port);
                if effective_port.is_none() {
                    // Both defaults disabled and no explicit port — skip
                    continue;
                }

                for (filters, host) in &port.hosts {
                    // Build descriptive block name based on filters
                    let block_name = match (&filters.host, &filters.ip) {
                        (Some(hostname), Some(ip)) => {
                            format!(
                                "port {} host {} ip {}",
                                effective_port.unwrap(),
                                hostname,
                                ip
                            )
                        }
                        (Some(hostname), None) => {
                            format!("port {} host {}", effective_port.unwrap(), hostname)
                        }
                        (None, Some(ip)) => {
                            format!("port {} ip {}", effective_port.unwrap(), ip)
                        }
                        (None, None) => {
                            format!("port {}", effective_port.unwrap())
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

    fn register_per_protocol_configuration_validators(
        &mut self,
        registry: &mut std::collections::HashMap<
            &'static str,
            Vec<Box<dyn ferron_core::config::validator::ConfigurationValidator>>,
        >,
    ) {
        registry
            .entry("http")
            .or_default()
            .push(Box::new(HttpConfigurationValidator));
        registry
            .entry("http")
            .or_default()
            .push(Box::new(BuiltinConfigurationValidator));
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
            // entries. Localhost-like hostnames are excluded from the HTTPS listener.
            let mut expanded: Vec<ServerConfigurationPort> = Vec::new();
            for port_config in &port_configs {
                if port_config.port.is_some() {
                    // Explicit port — use as-is (no automatic TLS expansion)
                    expanded.push(port_config.clone());
                } else {
                    // No explicit port — expand based on default port settings
                    let mut http_hosts = Vec::new();
                    let mut https_hosts = Vec::new();

                    for (filters, block) in &port_config.hosts {
                        let hostname = filters.host.as_deref();
                        let ip = filters.ip.map(|s| s.to_string());
                        let auto_selection = crate::tls_auto::select_auto_tls_provider(
                            &registry,
                            hostname,
                            ip.as_deref(),
                        );

                        http_hosts.push((filters.clone(), block.clone()));
                        if auto_selection != crate::tls_auto::TlsAutoSelection::None
                            || block.directives.contains_key("tls")
                        {
                            https_hosts.push((filters.clone(), block.clone()));
                        }
                    }

                    // HTTP listener gets all hosts (including localhost) — only if default HTTP port is enabled
                    if let Some(http_port) = default_port {
                        if !http_hosts.is_empty() {
                            let mut http_config = port_config.clone();
                            http_config.port = Some(http_port);
                            http_config.hosts = http_hosts;
                            expanded.push(http_config);
                        }
                    }

                    // HTTPS listener only gets non-localhost hosts — only if default HTTPS port is enabled
                    if let Some(https_port) = default_https {
                        if !https_hosts.is_empty() {
                            let mut https_config = port_config.clone();
                            https_config.port = Some(https_port);
                            https_config.hosts = https_hosts;
                            expanded.push(https_config);
                        }
                    }

                    // Warn if neither default is enabled and no explicit port was set
                    if default_port.is_none() && default_https.is_none() {
                        ferron_core::log_warn!(
                            "Host block without explicit port will be skipped because both default_http_port and default_https_port are disabled"
                        );
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
                // If default_https_port is false, set to None to disable redirects.
                let is_explicit_port = port_configs.iter().any(|pc| pc.port == Some(port));
                let https_port = if is_explicit_port {
                    Some(port) // Same port → redirect stage will skip
                } else {
                    default_https // May be None if default_https_port false
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

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::config::{
        ServerConfiguration, ServerConfigurationBlock, ServerConfigurationDirectiveEntry,
        ServerConfigurationHostFilters, ServerConfigurationPort, ServerConfigurationValue,
    };
    use std::collections::BTreeMap;
    use std::collections::HashMap as StdHashMap;
    use std::sync::Arc;

    fn make_config_with_directives(
        directives: StdHashMap<String, Vec<ServerConfigurationDirectiveEntry>>,
        ports: BTreeMap<String, Vec<ServerConfigurationPort>>,
    ) -> Arc<ServerConfiguration> {
        Arc::new(ServerConfiguration {
            global_config: Arc::new(ServerConfigurationBlock {
                directives: Arc::new(directives),
                matchers: StdHashMap::new(),
                span: None,
            }),
            ports,
        })
    }

    fn make_host_block(
        hostname: Option<&str>,
        directives: StdHashMap<String, Vec<ServerConfigurationDirectiveEntry>>,
    ) -> (ServerConfigurationHostFilters, ServerConfigurationBlock) {
        let filters = ServerConfigurationHostFilters {
            host: hostname.map(|s| s.to_string()),
            ip: None,
        };
        let block = ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: StdHashMap::new(),
            span: None,
        };
        (filters, block)
    }

    #[test]
    fn test_resolve_default_http_port_number() {
        let mut directives = StdHashMap::new();
        directives.insert(
            "default_http_port".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::Number(8080, None)],
                children: None,
                span: None,
            }],
        );
        let config = make_config_with_directives(directives, BTreeMap::new());
        assert_eq!(resolve_default_http_port(&config), Some(8080));
    }

    #[test]
    fn test_resolve_default_http_port_false() {
        let mut directives = StdHashMap::new();
        directives.insert(
            "default_http_port".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::Boolean(false, None)],
                children: None,
                span: None,
            }],
        );
        let config = make_config_with_directives(directives, BTreeMap::new());
        assert_eq!(resolve_default_http_port(&config), None);
    }

    #[test]
    fn test_resolve_default_http_port_true() {
        let mut directives = StdHashMap::new();
        directives.insert(
            "default_http_port".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::Boolean(true, None)],
                children: None,
                span: None,
            }],
        );
        let config = make_config_with_directives(directives, BTreeMap::new());
        assert_eq!(resolve_default_http_port(&config), Some(DEFAULT_HTTP_PORT));
    }

    #[test]
    fn test_resolve_default_http_port_missing() {
        let config = make_config_with_directives(StdHashMap::new(), BTreeMap::new());
        assert_eq!(resolve_default_http_port(&config), Some(DEFAULT_HTTP_PORT));
    }

    #[test]
    fn test_resolve_default_https_port_number() {
        let mut directives = StdHashMap::new();
        directives.insert(
            "default_https_port".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::Number(8443, None)],
                children: None,
                span: None,
            }],
        );
        let config = make_config_with_directives(directives, BTreeMap::new());
        assert_eq!(resolve_default_https_port(&config), Some(8443));
    }

    #[test]
    fn test_resolve_default_https_port_false() {
        let mut directives = StdHashMap::new();
        directives.insert(
            "default_https_port".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::Boolean(false, None)],
                children: None,
                span: None,
            }],
        );
        let config = make_config_with_directives(directives, BTreeMap::new());
        assert_eq!(resolve_default_https_port(&config), None);
    }

    #[test]
    fn test_register_blocks_with_disabled_defaults() {
        // Test that host blocks without explicit ports are skipped when both defaults are false
        let mut directives = StdHashMap::new();
        directives.insert(
            "default_http_port".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::Boolean(false, None)],
                children: None,
                span: None,
            }],
        );
        directives.insert(
            "default_https_port".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::Boolean(false, None)],
                children: None,
                span: None,
            }],
        );

        let mut ports = BTreeMap::new();
        let host = make_host_block(Some("example.com"), StdHashMap::new());
        ports.insert(
            "http".to_string(),
            vec![ServerConfigurationPort {
                port: None,
                hosts: vec![host],
            }],
        );

        let config = make_config_with_directives(directives, ports);
        let mut loader = BasicHttpModuleLoader::default();
        let mut registry = StdHashMap::new();

        loader.register_per_protocol_configuration_blocks(&config, &mut registry);

        // Should be empty because both defaults are disabled and no explicit port
        assert!(registry.is_empty() || registry.get("http").is_none_or(|v| v.is_empty()));
    }

    #[test]
    fn test_register_blocks_with_explicit_port_and_disabled_defaults() {
        // Test that explicit ports still work when defaults are disabled
        let mut directives = StdHashMap::new();
        directives.insert(
            "default_http_port".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::Boolean(false, None)],
                children: None,
                span: None,
            }],
        );
        directives.insert(
            "default_https_port".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::Boolean(false, None)],
                children: None,
                span: None,
            }],
        );

        let mut ports = BTreeMap::new();
        let host = make_host_block(Some("example.com"), StdHashMap::new());
        ports.insert(
            "http".to_string(),
            vec![ServerConfigurationPort {
                port: Some(9090),
                hosts: vec![host],
            }],
        );

        let config = make_config_with_directives(directives, ports);
        let mut loader = BasicHttpModuleLoader::default();
        let mut registry = StdHashMap::new();

        loader.register_per_protocol_configuration_blocks(&config, &mut registry);

        // Should have one block for the explicit port
        let blocks = registry.get("http").expect("http key should exist");
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].0.contains("port 9090"));
    }

    #[test]
    fn test_register_blocks_with_http_enabled_https_disabled() {
        // Test that only HTTP listener is created when HTTPS is disabled
        let mut directives = StdHashMap::new();
        directives.insert(
            "default_https_port".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::Boolean(false, None)],
                children: None,
                span: None,
            }],
        );

        let mut ports = BTreeMap::new();
        let host = make_host_block(Some("example.com"), StdHashMap::new());
        ports.insert(
            "http".to_string(),
            vec![ServerConfigurationPort {
                port: None,
                hosts: vec![host],
            }],
        );

        let config = make_config_with_directives(directives, ports);
        let mut loader = BasicHttpModuleLoader::default();
        let mut registry = StdHashMap::new();

        loader.register_per_protocol_configuration_blocks(&config, &mut registry);

        // Should have one block for HTTP on default port 80
        let blocks = registry.get("http").expect("http key should exist");
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].0.contains("port 80"));
    }
}
