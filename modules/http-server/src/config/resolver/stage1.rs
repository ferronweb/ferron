use std::{collections::HashMap, net::IpAddr, sync::Arc};

use ferron_core::config::{layer::LayeredConfiguration, ServerConfigurationBlock};

use super::super::prepare::HostConfigs;
use super::super::prepare::PreparedHostConfigurationBlock;
use super::types::ResolvedLocationPath;

/// Stage 1 resolver: IP address-based configuration lookup
///
/// Uses a BTreeMap for ordered IP address lookups.
#[derive(Debug, Default)]
pub struct Stage1IpResolver {
    /// Maps IP addresses to host configurations (named + default)
    ip_map: std::collections::BTreeMap<IpAddr, HostConfigs>,
    /// Default host configurations when no IP matches
    default: Option<HostConfigs>,
}

impl Stage1IpResolver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a configuration for a specific IP address
    pub fn register_ip(&mut self, ip: IpAddr, hosts: HostConfigs) {
        self.ip_map.insert(ip, hosts);
    }

    /// Set the default configuration when no IP matches
    pub fn set_default(&mut self, hosts: HostConfigs) {
        self.default = Some(hosts);
    }

    /// Resolve configuration for an IP address
    ///
    /// Returns the matched host configurations and updates the location path
    pub fn resolve(
        &self,
        ip: IpAddr,
        location_path: &mut ResolvedLocationPath,
    ) -> Option<&HostConfigs> {
        location_path.ip = Some(ip);

        if let Some(config) = self.ip_map.get(&ip) {
            return Some(config);
        }

        self.default.as_ref()
    }

    /// Look up a specific host within an IP's host configurations
    ///
    /// Returns the host configuration for the given hostname, falling back
    /// to the default host if no named match is found.
    /// Uses `&str` lookup — no `String` allocation.
    pub fn resolve_host(
        &self,
        ip: IpAddr,
        hostname: &str,
        location_path: &mut ResolvedLocationPath,
    ) -> Option<&Arc<PreparedHostConfigurationBlock>> {
        location_path.ip = Some(ip);

        let hosts = self.ip_map.get(&ip).or(self.default.as_ref())?;
        hosts.get(hostname)
    }

    /// Resolve and create a layered configuration
    ///
    /// # Arguments
    /// * `ip` - Client IP address to resolve
    /// * `base_config` - Optional base layered configuration to add layers to
    pub fn resolve_layered(
        &self,
        ip: IpAddr,
        base_config: Option<LayeredConfiguration>,
    ) -> (LayeredConfiguration, ResolvedLocationPath) {
        let mut location_path = ResolvedLocationPath::new();
        let mut layered_config = base_config.unwrap_or_default();

        if let Some(hosts) = self.resolve(ip, &mut location_path) {
            // Add the default host configuration if available
            if let Some(default_host) = hosts.get_default() {
                // Clone the Arc (cheap - just increments ref count, no HashMap clone)
                let block = ServerConfigurationBlock {
                    directives: Arc::clone(&default_host.directives),
                    matchers: HashMap::new(),
                    span: None,
                };
                layered_config.add_layer(Arc::new(block));
            }
        }

        (layered_config, location_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn create_test_block() -> PreparedHostConfigurationBlock {
        PreparedHostConfigurationBlock {
            directives: Arc::new(HashMap::new()),
            matches: Vec::new(),
            error_config: Vec::new(),
        }
    }

    #[test]
    fn test_stage1_ip_resolver() {
        let mut resolver = Stage1IpResolver::new();

        let mut hosts = HostConfigs::new();
        hosts.insert(
            Some("example.com".to_string()),
            Arc::new(create_test_block()),
        );

        resolver.register_ip("127.0.0.1".parse().unwrap(), hosts);

        let mut path = ResolvedLocationPath::new();
        let result = resolver.resolve("127.0.0.1".parse().unwrap(), &mut path);

        assert!(result.is_some());
        assert_eq!(path.ip, Some("127.0.0.1".parse().unwrap()));
    }

    #[test]
    fn test_stage1_resolve_host() {
        let mut resolver = Stage1IpResolver::new();

        let mut hosts = HostConfigs::new();
        hosts.insert(
            Some("example.com".to_string()),
            Arc::new(create_test_block()),
        );
        hosts.insert(None, Arc::new(create_test_block()));

        resolver.register_ip("127.0.0.1".parse().unwrap(), hosts);

        let mut path = ResolvedLocationPath::new();
        let result = resolver.resolve_host("127.0.0.1".parse().unwrap(), "example.com", &mut path);

        assert!(result.is_some());
        assert_eq!(path.ip, Some("127.0.0.1".parse().unwrap()));
    }

    #[test]
    fn test_stage1_layered_resolution() {
        let mut resolver = Stage1IpResolver::new();

        let mut hosts = HostConfigs::new();
        hosts.insert(None, Arc::new(create_test_block()));

        resolver.register_ip("127.0.0.1".parse().unwrap(), hosts);

        let (config, path) = resolver.resolve_layered("127.0.0.1".parse().unwrap(), None);

        assert_eq!(path.ip, Some("127.0.0.1".parse().unwrap()));
        assert_eq!(config.layers.len(), 1);
    }
}
