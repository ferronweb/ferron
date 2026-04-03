use std::{collections::HashMap, net::IpAddr, sync::Arc};

use ferron_core::config::{layer::LayeredConfiguration, ServerConfigurationBlock};

use super::super::prepare::PreparedHostConfigurationBlock;
use super::types::ResolvedLocationPath;

/// Stage 1 resolver: IP address-based configuration lookup
///
/// Uses a BTreeMap for ordered IP address lookups.
#[derive(Debug)]
pub struct Stage1IpResolver {
    /// Maps IP addresses to prepared host configurations
    ip_map:
        std::collections::BTreeMap<IpAddr, HashMap<Option<String>, PreparedHostConfigurationBlock>>,
    /// Default configuration when no IP matches
    default: Option<HashMap<Option<String>, PreparedHostConfigurationBlock>>,
}

impl Stage1IpResolver {
    pub fn new() -> Self {
        Self {
            ip_map: std::collections::BTreeMap::new(),
            default: None,
        }
    }

    /// Register a configuration for a specific IP address
    pub fn register_ip(
        &mut self,
        ip: IpAddr,
        hosts: HashMap<Option<String>, PreparedHostConfigurationBlock>,
    ) {
        self.ip_map.insert(ip, hosts);
    }

    /// Set the default configuration when no IP matches
    pub fn set_default(&mut self, hosts: HashMap<Option<String>, PreparedHostConfigurationBlock>) {
        self.default = Some(hosts);
    }

    /// Resolve configuration for an IP address
    ///
    /// Returns the matched host configurations and updates the location path
    pub fn resolve(
        &self,
        ip: IpAddr,
        location_path: &mut ResolvedLocationPath,
    ) -> Option<&HashMap<Option<String>, PreparedHostConfigurationBlock>> {
        location_path.ip = Some(ip);

        if let Some(config) = self.ip_map.get(&ip) {
            return Some(config);
        }

        self.default.as_ref()
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
            if let Some(default_host) = hosts.get(&None) {
                // Clone the Arc (cheap - just increments ref count)
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

impl Default for Stage1IpResolver {
    fn default() -> Self {
        Self::new()
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

        let mut hosts = HashMap::new();
        hosts.insert(Some("example.com".to_string()), create_test_block());

        resolver.register_ip("127.0.0.1".parse().unwrap(), hosts);

        let mut path = ResolvedLocationPath::new();
        let result = resolver.resolve("127.0.0.1".parse().unwrap(), &mut path);

        assert!(result.is_some());
        assert_eq!(path.ip, Some("127.0.0.1".parse().unwrap()));
    }

    #[test]
    fn test_stage1_layered_resolution() {
        let mut resolver = Stage1IpResolver::new();

        let mut hosts = HashMap::new();
        let host_block = create_test_block();
        // Use None as the key (default host config)
        hosts.insert(None, host_block);

        resolver.register_ip("127.0.0.1".parse().unwrap(), hosts);

        let (config, path) = resolver.resolve_layered("127.0.0.1".parse().unwrap(), None);

        assert_eq!(path.ip, Some("127.0.0.1".parse().unwrap()));
        assert_eq!(config.layers.len(), 1);
    }
}
