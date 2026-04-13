#![allow(dead_code)]
//! 3-Stage Configuration Resolver
//!
//! This module provides a modular configuration resolution system with three independent stages:
//!
//! 1. **Stage 1** - IP address-based resolution (BTreeMap)
//! 2. **Stage 2** - Main resolution using radix tree (hostname segments, wildcards, path segments, conditionals)
//! 3. **Stage 3** - Error configuration resolution (HashMap)
//!
//! Each stage can be used independently or composed together via the main resolver.

use std::{collections::HashMap, net::IpAddr, sync::Arc};

use ferron_core::config::{layer::LayeredConfiguration, ServerConfigurationBlock};
use ferron_http::HttpContext;

use super::super::prepare::{
    HostConfigs, PreparedConfiguration, PreparedHostConfigurationBlock,
    PreparedHostConfigurationErrorConfig, PreparedHostConfigurationMatch,
    PreparedHostConfigurationMatcher,
};
use super::stage1::Stage1IpResolver;
use super::stage2::Stage2RadixResolver;
use super::stage3::{ConditionalGroup, ErrorConfigScope, Stage3ErrorResolver};
use super::types::{ResolutionResult, ResolvedLocationPath};

pub struct ThreeStageResolver {
    global: Option<Arc<ServerConfigurationBlock>>,
    stage1_ip: Stage1IpResolver,
    stage2_radix: Stage2RadixResolver,
    stage3_error: Stage3ErrorResolver,
}

impl ThreeStageResolver {
    pub fn new() -> Self {
        Self {
            global: None,
            stage1_ip: Stage1IpResolver::new(),
            stage2_radix: Stage2RadixResolver::new(),
            stage3_error: Stage3ErrorResolver::new(),
        }
    }

    /// Create a resolver from prepared configuration
    ///
    /// Populates all three stages:
    /// - Stage 1: IP-based host configuration mapping
    /// - Stage 2: Hostname radix tree with location/conditional matchers
    /// - Stage 3: Error configuration lookup (with scoped support from nested configs)
    pub fn from_prepared(prepared: PreparedConfiguration) -> Self {
        let mut resolver = Self::new();

        for (ip_opt, hosts) in prepared {
            // Stage 1: Register IP -> hosts mapping
            if let Some(ip) = ip_opt {
                resolver.stage1_ip.register_ip(ip, hosts.clone());
            } else {
                resolver.stage1_ip.set_default(hosts.clone());
            }

            // Stage 2 & 3: Register hostname configs and error configs
            for (hostname_opt, host_arc) in &hosts.named_hosts {
                // Stage 2: Insert hostname into radix tree
                let segments: Vec<&str> = hostname_opt.split('.').rev().collect();
                resolver
                    .stage2_radix
                    .insert_host(segments, Arc::clone(host_arc), 10);

                // Stage 3: Register error configs recursively with proper scopes
                Self::register_error_configs_stage3(
                    &mut resolver.stage2_radix,
                    &mut resolver.stage3_error,
                    Some(hostname_opt.as_str()),
                    host_arc,
                    Vec::new(), // no accumulated conditionals at host level
                    None,       // no path at host level
                );

                // Stage 2: Register IfConditional and IfNotConditional matchers
                Self::register_matchers_stage2(&mut resolver.stage2_radix, &host_arc.matches);
            }

            // Handle default host (hostname = None)
            if let Some(ref default_arc) = hosts.default_host {
                Self::register_error_configs_stage3(
                    &mut resolver.stage2_radix,
                    &mut resolver.stage3_error,
                    None,
                    default_arc,
                    Vec::new(),
                    None,
                );
                Self::register_matchers_stage2(&mut resolver.stage2_radix, &default_arc.matches);
            }
        }

        resolver
    }

    /// Register IfConditional, IfNotConditional, and Location matchers into Stage 2
    fn register_matchers_stage2(
        stage2: &mut Stage2RadixResolver,
        matches: &[PreparedHostConfigurationMatch],
    ) {
        for m in matches {
            match &m.matcher {
                PreparedHostConfigurationMatcher::IfConditional(exprs) => {
                    let _ = stage2.insert_if_conditional(exprs.clone(), Arc::clone(&m.config), 10);
                }
                PreparedHostConfigurationMatcher::IfNotConditional(exprs) => {
                    let _ =
                        stage2.insert_if_not_conditional(exprs.clone(), Arc::clone(&m.config), 10);
                }
                PreparedHostConfigurationMatcher::Location(path_pattern) => {
                    // Insert location path into the path radix tree
                    let path_segments: Vec<&str> = path_pattern
                        .trim_start_matches('/')
                        .split('/')
                        .filter(|s| !s.is_empty())
                        .collect();
                    let priority = path_pattern.len() as u32;
                    stage2.insert_location(path_segments, Arc::clone(&m.config), priority);
                }
            }
        }
    }

    /// Recursively walk all match blocks and register their error configs into Stage 3
    /// with proper scope (hostname + path + accumulated conditional groups).
    /// Also registers location matchers into Stage 2.
    ///
    /// `if` blocks add a positive group, `if_not` blocks add a negated group.
    fn register_error_configs_stage3(
        stage2: &mut Stage2RadixResolver,
        stage3: &mut Stage3ErrorResolver,
        hostname: Option<&str>,
        block: &Arc<PreparedHostConfigurationBlock>,
        accumulated_groups: Vec<ConditionalGroup>,
        current_path: Option<String>,
    ) {
        // Register host-level error configs
        for error_config in &block.error_config {
            let config = Arc::new(error_config.config.clone());
            Self::register_single_error_config(
                stage3,
                hostname,
                current_path.as_deref(),
                &accumulated_groups,
                error_config.error_code,
                config,
            );
        }

        // Recursively walk match blocks
        for location_match in &block.matches {
            match &location_match.matcher {
                PreparedHostConfigurationMatcher::Location(ref path_pattern) => {
                    // Register location into Stage 2 radix tree
                    let path_segments: Vec<&str> = path_pattern
                        .trim_start_matches('/')
                        .split('/')
                        .filter(|s| !s.is_empty())
                        .collect();
                    let priority = path_pattern.len() as u32;
                    stage2.insert_location(
                        path_segments,
                        Arc::clone(&location_match.config),
                        priority,
                    );

                    // Register location-level error configs
                    for error_config in &location_match.config.error_config {
                        let config = Arc::new(error_config.config.clone());
                        Self::register_single_error_config(
                            stage3,
                            hostname,
                            Some(path_pattern.as_str()),
                            &accumulated_groups,
                            error_config.error_code,
                            config,
                        );
                    }
                    // Recurse into nested location match blocks with the path
                    Self::register_error_configs_stage3(
                        stage2,
                        stage3,
                        hostname,
                        &location_match.config,
                        accumulated_groups.clone(),
                        Some(path_pattern.clone()),
                    );
                }
                PreparedHostConfigurationMatcher::IfConditional(ref exprs) => {
                    // Add a positive conditional group
                    let mut new_groups = accumulated_groups.clone();
                    new_groups.push(ConditionalGroup {
                        exprs: exprs.clone(),
                        negated: false,
                    });
                    // Register error configs inside this if block
                    for error_config in &location_match.config.error_config {
                        let config = Arc::new(error_config.config.clone());
                        Self::register_single_error_config(
                            stage3,
                            hostname,
                            current_path.as_deref(),
                            &new_groups,
                            error_config.error_code,
                            config,
                        );
                    }
                    // Recurse into nested match blocks inside this if
                    Self::register_error_configs_stage3(
                        stage2,
                        stage3,
                        hostname,
                        &location_match.config,
                        new_groups,
                        current_path.clone(),
                    );
                }
                PreparedHostConfigurationMatcher::IfNotConditional(ref exprs) => {
                    // Add a negated conditional group
                    let mut new_groups = accumulated_groups.clone();
                    new_groups.push(ConditionalGroup {
                        exprs: exprs.clone(),
                        negated: true,
                    });
                    // Register error configs inside this if_not block
                    for error_config in &location_match.config.error_config {
                        let config = Arc::new(error_config.config.clone());
                        Self::register_single_error_config(
                            stage3,
                            hostname,
                            current_path.as_deref(),
                            &new_groups,
                            error_config.error_code,
                            config,
                        );
                    }
                    // Recurse into nested match blocks inside this if_not
                    Self::register_error_configs_stage3(
                        stage2,
                        stage3,
                        hostname,
                        &location_match.config,
                        new_groups,
                        current_path.clone(),
                    );
                }
            }
        }
    }

    /// Register a single error config with the proper scope
    fn register_single_error_config(
        stage3: &mut Stage3ErrorResolver,
        hostname: Option<&str>,
        path: Option<&str>,
        conditionals: &[ConditionalGroup],
        error_code: Option<u16>,
        config: Arc<PreparedHostConfigurationBlock>,
    ) {
        let scope = ErrorConfigScope {
            ip: None,
            hostname: hostname.map(|s| s.to_string()),
            path: path.map(|s| s.to_string()),
            conditionals: conditionals.to_vec(),
            error_code,
        };

        if !conditionals.is_empty() {
            stage3.register(scope, config);
        } else if let Some(hostname) = hostname {
            if let Some(code) = error_code {
                if let Some(path) = path {
                    stage3.register_hostname_path_error(hostname, path, code, config);
                } else {
                    stage3.register_hostname_error(hostname, code, config);
                }
            } else {
                if let Some(path) = path {
                    stage3.set_hostname_path_default(hostname, path, config);
                } else {
                    stage3.set_hostname_default(hostname, config);
                }
            }
        } else {
            if let Some(code) = error_code {
                if let Some(path) = path {
                    stage3.register_path_error(path, code, config);
                } else {
                    stage3.register_error(code, config);
                }
            } else {
                if let Some(path) = path {
                    stage3.set_path_default(path, config);
                } else {
                    stage3.set_default(config);
                }
            }
        }
    }

    /// Create a resolver from prepared configuration and global configuration
    pub fn from_prepared_with_global(
        prepared: PreparedConfiguration,
        global: Arc<ServerConfigurationBlock>,
    ) -> Self {
        let mut resolver = Self::from_prepared(prepared);
        resolver.global = Some(global);
        resolver
    }

    /// Get mutable reference to Stage 1 resolver
    pub fn stage1(&mut self) -> &mut Stage1IpResolver {
        &mut self.stage1_ip
    }

    /// Get mutable reference to Stage 2 resolver
    pub fn stage2(&mut self) -> &mut Stage2RadixResolver {
        &mut self.stage2_radix
    }

    /// Get mutable reference to Stage 3 resolver
    pub fn stage3(&mut self) -> &mut Stage3ErrorResolver {
        &mut self.stage3_error
    }

    /// Get immutable reference to Stage 1 resolver
    pub fn stage1_ref(&self) -> &Stage1IpResolver {
        &self.stage1_ip
    }

    /// Get immutable reference to Stage 2 resolver
    pub fn stage2_ref(&self) -> &Stage2RadixResolver {
        &self.stage2_radix
    }

    /// Get immutable reference to Stage 3 resolver
    pub fn stage3_ref(&self) -> &Stage3ErrorResolver {
        &self.stage3_error
    }

    /// Full resolution through all stages
    ///
    /// # Arguments
    /// * `ip` - Client IP address for Stage 1
    /// * `hostname` - Request hostname for Stage 2
    /// * `path` - Request path for Stage 2
    /// * `ctx` - HTTP context for conditional evaluation in Stage 2
    pub fn resolve(
        &self,
        ip: IpAddr,
        hostname: &str,
        path: &str,
        ctx: &HttpContext,
    ) -> Option<ResolutionResult> {
        let mut location_path = ResolvedLocationPath::new();

        // Stage 1: IP-based resolution (zero-copy Arc lookup)
        let host_config_arc = self
            .stage1_ip
            .resolve_host(ip, hostname, &mut location_path)?;

        // Stage 2: Hostname, path, and conditional resolution (passing Stage 1's config)
        let (stage2_config, stage2_path) =
            self.stage2_radix
                .resolve(Some(hostname), path, Arc::clone(host_config_arc), ctx, None);

        // Merge Stage 2 results
        let mut layered_config = LayeredConfiguration::new();
        if let Some(global) = self.global.clone() {
            layered_config.add_layer(global);
        }
        for layer in stage2_config.layers {
            layered_config.add_layer(layer);
        }
        location_path.hostname_segments = stage2_path.hostname_segments;
        location_path.path_segments = stage2_path.path_segments;
        location_path.conditionals = stage2_path.conditionals;

        Some(ResolutionResult::new(layered_config, location_path))
    }

    /// Resolve only through Stage 1 (IP-based)
    ///
    /// Returns the host configurations for the given IP
    pub fn resolve_stage1(&self, ip: IpAddr) -> Option<&HostConfigs> {
        let mut path = ResolvedLocationPath::new();
        self.stage1_ip.resolve(ip, &mut path)
    }

    /// Resolve only through Stage 1 (IP-based) and return layered configuration
    ///
    /// # Arguments
    /// * `ip` - Client IP address to resolve
    /// * `base_config` - Optional base layered configuration to add layers to
    pub fn resolve_stage1_layered(
        &self,
        ip: IpAddr,
        base_config: Option<LayeredConfiguration>,
    ) -> (LayeredConfiguration, ResolvedLocationPath) {
        self.stage1_ip.resolve_layered(ip, base_config)
    }

    /// Resolve only through Stage 2 (hostname/path/conditionals)
    ///
    /// # Arguments
    /// * `hostname` - Request hostname to resolve
    /// * `path` - Request path to resolve
    /// * `base_config` - The base prepared host configuration block (Arc for zero-copy sharing)
    /// * `ctx` - HTTP context for conditional evaluation
    pub fn resolve_stage2(
        &self,
        hostname: Option<&str>,
        path: &str,
        base_config: Arc<PreparedHostConfigurationBlock>,
        ctx: &HttpContext,
    ) -> (LayeredConfiguration, ResolvedLocationPath) {
        self.stage2_radix
            .resolve(hostname, path, base_config, ctx, None)
    }

    /// Resolve only through Stage 2 (hostname/path/conditionals) with base layered config
    ///
    /// # Arguments
    /// * `hostname` - Request hostname to resolve
    /// * `path` - Request path to resolve
    /// * `base_config` - The base prepared host configuration block (Arc for zero-copy sharing)
    /// * `ctx` - HTTP context for conditional evaluation
    /// * `layered_config` - Optional base layered configuration to add layers to
    pub fn resolve_stage2_layered(
        &self,
        hostname: Option<&str>,
        path: &str,
        base_config: Arc<PreparedHostConfigurationBlock>,
        ctx: &HttpContext,
        layered_config: Option<LayeredConfiguration>,
    ) -> (LayeredConfiguration, ResolvedLocationPath) {
        self.stage2_radix
            .resolve(hostname, path, base_config, ctx, layered_config)
    }

    /// Resolve only through Stage 3 (error configuration)
    ///
    /// # Arguments
    /// * `error_code` - Error code to resolve
    pub fn resolve_stage3(&self, error_code: u16) -> (LayeredConfiguration, ResolvedLocationPath) {
        self.stage3_error.resolve_layered(error_code, None)
    }

    /// Resolve only through Stage 3 (error configuration) with base layered config
    ///
    /// # Arguments
    /// * `error_code` - Error code to resolve
    /// * `layered_config` - Optional base layered configuration to add layers to
    pub fn resolve_stage3_layered(
        &self,
        error_code: u16,
        layered_config: Option<LayeredConfiguration>,
    ) -> (LayeredConfiguration, ResolvedLocationPath) {
        self.stage3_error
            .resolve_layered(error_code, layered_config)
    }

    /// Resolve error configuration for a specific error code
    ///
    /// This method resolves through all stages (IP, hostname/path, error) and applies
    /// the error configuration layer on top of the base configuration.
    ///
    /// # Arguments
    /// * `ip` - Client IP address for Stage 1
    /// * `hostname` - Request hostname for Stage 2
    /// * `path` - Request path for Stage 2
    /// * `error_code` - Error code for Stage 3
    /// * `ctx` - HTTP context for conditional evaluation
    pub fn resolve_error(
        &self,
        ip: IpAddr,
        hostname: &str,
        path: &str,
        error_code: u16,
        ctx: &HttpContext,
    ) -> Option<ResolutionResult> {
        let mut location_path = ResolvedLocationPath::new();

        // Stage 1: IP-based resolution (zero-copy Arc lookup)
        let host_config_arc = self
            .stage1_ip
            .resolve_host(ip, hostname, &mut location_path)?;

        // Stage 2: Hostname, path, and conditional resolution
        let (stage2_config, stage2_path) =
            self.stage2_radix
                .resolve(Some(hostname), path, Arc::clone(host_config_arc), ctx, None);

        // Merge Stage 2 results
        let mut layered_config = LayeredConfiguration::new();
        if let Some(global) = self.global.clone() {
            layered_config.add_layer(global);
        }
        for layer in stage2_config.layers {
            layered_config.add_layer(layer);
        }
        location_path.hostname_segments = stage2_path.hostname_segments;
        location_path.path_segments = stage2_path.path_segments;
        location_path.conditionals = stage2_path.conditionals;

        // Stage 3: Error configuration resolution
        let error_config = self.stage3_error.resolve(error_code, &mut location_path);
        if let Some(error_block) = error_config {
            let block = ServerConfigurationBlock {
                directives: Arc::clone(&error_block.directives),
                matchers: HashMap::new(),
                span: None,
            };
            layered_config.add_layer(Arc::new(block));
        }

        Some(ResolutionResult::new(layered_config, location_path))
    }

    /// Resolve error configuration with full Stage 2 → Stage 3 chaining
    ///
    /// This method properly chains Stage 3 on top of Stage 2's base configuration,
    /// using scoped error resolution based on hostname, IP, and path context.
    ///
    /// Resolution order for error configs (most specific to least specific):
    /// 1. Path-specific error config
    /// 2. Hostname-specific error config
    /// 3. IP-specific error config
    /// 4. Global error config
    /// 5. Scoped default (path → hostname → IP → global)
    ///
    /// # Arguments
    /// * `ip` - Client IP address for Stage 1
    /// * `hostname` - Request hostname for Stage 2
    /// * `path` - Request path for Stage 2
    /// * `error_code` - Error code for Stage 3
    /// * `ctx` - HTTP context for conditional evaluation
    pub fn resolve_error_scoped(
        &self,
        ip: IpAddr,
        hostname: &str,
        path: &str,
        error_code: u16,
        ctx: &HttpContext,
    ) -> Option<ResolutionResult> {
        let mut location_path = ResolvedLocationPath::new();

        // Stage 1: IP-based resolution (zero-copy Arc lookup)
        let host_config_arc = self
            .stage1_ip
            .resolve_host(ip, hostname, &mut location_path)?;

        // Stage 2: Hostname, path, and conditional resolution
        let (stage2_config, stage2_path) =
            self.stage2_radix
                .resolve(Some(hostname), path, Arc::clone(host_config_arc), ctx, None);

        // Merge Stage 2 results
        let mut layered_config = LayeredConfiguration::new();
        if let Some(global) = self.global.clone() {
            layered_config.add_layer(global);
        }
        for layer in stage2_config.layers {
            layered_config.add_layer(layer);
        }
        location_path.hostname_segments = stage2_path.hostname_segments;
        location_path.path_segments = stage2_path.path_segments;
        location_path.conditionals = stage2_path.conditionals;

        // Stage 3: Error configuration resolution with full scoping
        // Pass hostname, IP, and path_segments for scoped lookup
        let (error_layered_config, error_path) = self.stage3_error.resolve_layered_scoped(
            error_code,
            Some(hostname),
            Some(ip),
            Some(&location_path.path_segments),
            ctx,
            Some(layered_config),
        );

        // Merge error path info
        location_path.error_key = error_path.error_key;

        Some(ResolutionResult::new(error_layered_config, location_path))
    }

    /// Get the global configuration block
    pub fn global(&self) -> Option<Arc<ServerConfigurationBlock>> {
        self.global.clone()
    }
}

impl Default for ThreeStageResolver {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use ferron_core::config::layer::LayeredConfiguration;
    use ferron_core::config::{
        ServerConfigurationMatcherExpr, ServerConfigurationMatcherOperand,
        ServerConfigurationMatcherOperator,
    };
    use ferron_http::HttpContext;
    use ferron_http::HttpRequest;
    use ferron_observability::CompositeEventSink;
    use http_body_util::{BodyExt, Empty};
    use rustc_hash::FxHashMap;
    use typemap_rev::TypeMap;

    use super::*;
    use crate::config::prepare::{
        prepare_host_config, HostConfigs, PreparedHostConfigurationBlock,
    };
    use crate::config::resolver::matcher::CompiledMatcherExpr;
    use std::net::Ipv4Addr;

    fn make_test_context(req: HttpRequest) -> HttpContext {
        HttpContext {
            req: Some(req),
            res: None,
            events: CompositeEventSink::new(Vec::new()),
            configuration: LayeredConfiguration::default(),
            hostname: Some("example.com".to_string()),
            variables: FxHashMap::default(),
            previous_error: None,
            original_uri: None,
            routing_uri: None,
            encrypted: false,
            local_address: "0.0.0.0:80".parse().unwrap(),
            remote_address: "127.0.0.1:12345".parse().unwrap(),
            auth_user: None,
            https_port: None,
            extensions: TypeMap::new(),
        }
    }

    fn create_test_block() -> PreparedHostConfigurationBlock {
        PreparedHostConfigurationBlock {
            directives: Arc::new(HashMap::new()),
            matches: Vec::new(),
            error_config: Vec::new(),
        }
    }

    #[test]
    fn test_stage2_layered_resolution() {
        let mut resolver = Stage2RadixResolver::new();

        let config = Arc::new(create_test_block());
        resolver.insert_host(vec!["com", "example"], Arc::clone(&config), 10);

        let base_block = create_test_block();
        let request = http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync());
        let ctx = make_test_context(request);
        let (layered_config, path) = resolver.resolve(
            Some("example.com"),
            "/api",
            Arc::new(base_block),
            &ctx,
            None,
        );

        assert!(!path.hostname_segments.is_empty());
        assert!(!layered_config.layers.is_empty());
    }

    #[test]
    fn test_chained_layered_resolution() {
        let mut resolver = ThreeStageResolver::new();

        // Setup Stage 1
        let mut hosts = HostConfigs::new();
        let mut directives1 = HashMap::new();
        directives1.insert("stage1_directive".to_string(), vec![]);
        let host_block = PreparedHostConfigurationBlock {
            directives: Arc::new(directives1),
            matches: Vec::new(),
            error_config: Vec::new(),
        };
        hosts.insert(Some("example.com".to_string()), Arc::new(host_block));
        resolver
            .stage1()
            .register_ip("127.0.0.1".parse().unwrap(), hosts);

        // Setup Stage 2
        let mut directives2 = HashMap::new();
        directives2.insert("stage2_directive".to_string(), vec![]);
        let host_block2 = PreparedHostConfigurationBlock {
            directives: Arc::new(directives2),
            matches: Vec::new(),
            error_config: Vec::new(),
        };
        resolver
            .stage2()
            .insert_host(vec!["com", "example"], Arc::new(host_block2), 10);

        // Setup Stage 3
        let error_block = create_test_block();
        resolver.stage3().register_error(404, Arc::new(error_block));

        // Chain resolutions: Stage 1 -> Stage 2 -> Stage 3
        let (config1, _) = resolver.resolve_stage1_layered("127.0.0.1".parse().unwrap(), None);

        let host_block = resolver
            .resolve_stage1("127.0.0.1".parse().unwrap())
            .unwrap()
            .get("example.com")
            .unwrap();

        let request = http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync());
        let ctx = make_test_context(request);
        let (config2, _) = resolver.resolve_stage2_layered(
            Some("example.com"),
            "/api",
            Arc::clone(host_block),
            &ctx,
            Some(config1),
        );

        let (config3, path) = resolver.resolve_stage3_layered(404, Some(config2));

        // Should have layers from all 3 stages
        assert_eq!(path.error_key, Some(404));
        assert_eq!(config3.layers.len(), 3);
    }

    #[test]
    fn test_three_stage_resolver() {
        let mut resolver = ThreeStageResolver::new();

        // Setup Stage 1
        let mut hosts = HostConfigs::new();
        let host_block = PreparedHostConfigurationBlock {
            directives: Arc::new(HashMap::new()),
            matches: Vec::new(),
            error_config: Vec::new(),
        };
        hosts.insert(Some("example.com".to_string()), Arc::new(host_block));

        resolver
            .stage1()
            .register_ip("127.0.0.1".parse().unwrap(), hosts);

        // Full resolution
        let request = http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync());
        let ctx = make_test_context(request);
        let result = resolver.resolve(
            "127.0.0.1".parse().unwrap(),
            "example.com",
            "/api/test",
            &ctx,
        );

        assert!(result.is_some());
        let result = result.unwrap();
        assert!(result.location_path.ip.is_some());
    }

    #[test]
    fn test_conditional_resolution() {
        use ferron_core::config::{
            ServerConfigurationMatcherExpr, ServerConfigurationMatcherOperand,
            ServerConfigurationMatcherOperator,
        };

        let mut resolver = Stage2RadixResolver::new();

        let config = Arc::new(create_test_block());

        // Add an if conditional
        let expr = ServerConfigurationMatcherExpr {
            left: ServerConfigurationMatcherOperand::Identifier("method".to_string()),
            right: ServerConfigurationMatcherOperand::String("GET".to_string()),
            op: ServerConfigurationMatcherOperator::Eq,
        };
        resolver
            .insert_if_conditional(vec![expr], config, 10)
            .expect("Valid conditional");

        let request = http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync());
        let mut ctx = make_test_context(request);
        ctx.variables
            .insert("method".to_string(), "GET".to_string());

        let mut path = ResolvedLocationPath::new();
        let configs = resolver.resolve_conditionals(&ctx, &mut path);

        assert!(!configs.is_empty());
        assert!(!path.conditionals.is_empty());
    }

    #[test]
    fn test_from_prepared_configuration() {
        use ferron_core::config::{
            ServerConfigurationBlock, ServerConfigurationHostFilters, ServerConfigurationPort,
        };

        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));

        let port = ServerConfigurationPort {
            port: Some(80),
            hosts: vec![(
                ServerConfigurationHostFilters {
                    ip: Some(ip),
                    host: Some("example.com".to_string()),
                },
                ServerConfigurationBlock {
                    directives: Arc::new(HashMap::new()),
                    matchers: HashMap::new(),
                    span: None,
                },
            )],
        };

        let prepared = prepare_host_config(port).unwrap();
        let resolver = ThreeStageResolver::from_prepared(prepared);

        assert!(resolver.resolve_stage1(ip).is_some());
    }

    /// Test that a single tree branch can carry multiple layered values:
    /// - Hostname-level configuration (from radix tree)
    /// - Location-level configuration (prefix match on path)
    /// - Conditional configuration (if directive)
    #[test]
    fn test_branch_with_multiple_layered_values() {
        use crate::config::prepare::{
            PreparedHostConfigurationBlock, PreparedHostConfigurationMatch,
            PreparedHostConfigurationMatcher,
        };
        use ferron_core::config::{
            ServerConfigurationDirectiveEntry, ServerConfigurationMatcherExpr,
            ServerConfigurationMatcherOperand, ServerConfigurationMatcherOperator,
            ServerConfigurationValue,
        };

        let mut resolver = Stage2RadixResolver::new();

        // Hostname-level config
        let mut host_directives = HashMap::new();
        host_directives.insert(
            "host_level".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String(
                    "hostname_value".to_string(),
                    None,
                )],
                children: None,
                span: None,
            }],
        );
        let host_config = Arc::new(PreparedHostConfigurationBlock {
            directives: Arc::new(host_directives),
            matches: Vec::new(),
            error_config: Vec::new(),
        });
        resolver.insert_host(vec!["com", "example"], Arc::clone(&host_config), 10);

        // Base block with location and conditional matchers
        let mut base_directives = HashMap::new();
        base_directives.insert(
            "base_level".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String(
                    "base_value".to_string(),
                    None,
                )],
                children: None,
                span: None,
            }],
        );
        let mut base_block = PreparedHostConfigurationBlock {
            directives: Arc::new(base_directives),
            matches: Vec::new(),
            error_config: Vec::new(),
        };

        // Location matcher: /api
        let mut loc_cfg = HashMap::new();
        loc_cfg.insert(
            "location_directive".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String(
                    "location_value".to_string(),
                    None,
                )],
                children: None,
                span: None,
            }],
        );
        let loc_config = Arc::new(PreparedHostConfigurationBlock {
            directives: Arc::new(loc_cfg),
            matches: Vec::new(),
            error_config: Vec::new(),
        });
        base_block.matches.push(PreparedHostConfigurationMatch {
            matcher: PreparedHostConfigurationMatcher::Location("/api".to_string()),
            config: Arc::clone(&loc_config),
        });

        // Register location into path tree
        resolver.insert_location(vec!["api"], loc_config, 4);

        // Conditional matcher: if method == GET
        let expr = ServerConfigurationMatcherExpr {
            left: ServerConfigurationMatcherOperand::Identifier("method".to_string()),
            right: ServerConfigurationMatcherOperand::String("GET".to_string()),
            op: ServerConfigurationMatcherOperator::Eq,
        };
        let mut cond_cfg = HashMap::new();
        cond_cfg.insert(
            "conditional_directive".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String(
                    "if_value".to_string(),
                    None,
                )],
                children: None,
                span: None,
            }],
        );
        base_block.matches.push(PreparedHostConfigurationMatch {
            matcher: PreparedHostConfigurationMatcher::IfConditional(vec![expr]),
            config: Arc::new(PreparedHostConfigurationBlock {
                directives: Arc::new(cond_cfg),
                matches: Vec::new(),
                error_config: Vec::new(),
            }),
        });

        // Resolve
        let request = http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync());
        let ctx = make_test_context(request);
        let (layered, path) = resolver.resolve(
            Some("example.com"),
            "/api/users",
            Arc::new(base_block),
            &ctx,
            None,
        );

        // Verify hostname matched
        assert!(!path.hostname_segments.is_empty());
        assert_eq!(path.hostname_segments, vec!["example", "com"]);

        // Verify location matched
        assert!(!path.path_segments.is_empty());
        assert_eq!(path.path_segments, vec!["api"]);

        // Verify multiple layers: hostname + base + location + conditional
        assert!(
            layered.layers.len() >= 3,
            "Expected >= 3 layers, got {}",
            layered.layers.len()
        );
    }

    #[test]
    fn test_regex_matcher_expr_matching() {
        use ferron_core::config::{
            ServerConfigurationMatcherExpr, ServerConfigurationMatcherOperand,
            ServerConfigurationMatcherOperator,
        };

        let mut resolver = Stage2RadixResolver::new();
        let config = Arc::new(create_test_block());

        // Create a regex matcher: if path matches /api/.*
        let regex_expr = ServerConfigurationMatcherExpr {
            left: ServerConfigurationMatcherOperand::Identifier("path".to_string()),
            right: ServerConfigurationMatcherOperand::String(r"^/api/.*".to_string()),
            op: ServerConfigurationMatcherOperator::Regex,
        };
        resolver
            .insert_if_conditional(vec![regex_expr], Arc::clone(&config), 10)
            .expect("Valid regex");

        // Create variables with matching path
        let request = http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync());
        let mut ctx = make_test_context(request);
        ctx.variables
            .insert("path".to_string(), "/api/users".to_string());

        let mut path = ResolvedLocationPath::new();
        let configs = resolver.resolve_conditionals(&ctx, &mut path);

        // Should match
        assert!(!configs.is_empty(), "Regex pattern should match /api/users");

        // Create variables with non-matching path
        let request2 = http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync());
        let mut ctx2 = make_test_context(request2);
        ctx2.variables
            .insert("path".to_string(), "/static/file.js".to_string());

        let mut path2 = ResolvedLocationPath::new();
        let configs2 = resolver.resolve_conditionals(&ctx2, &mut path2);

        // Should not match
        assert!(
            configs2.is_empty(),
            "Regex pattern should not match /static/file.js"
        );
    }

    #[test]
    fn test_not_regex_matcher_expr() {
        use ferron_core::config::{
            ServerConfigurationMatcherExpr, ServerConfigurationMatcherOperand,
            ServerConfigurationMatcherOperator,
        };

        let mut resolver = Stage2RadixResolver::new();
        let config = Arc::new(create_test_block());

        // Create a NOT regex matcher: if path does not match /admin/.*
        let not_regex_expr = ServerConfigurationMatcherExpr {
            left: ServerConfigurationMatcherOperand::Identifier("path".to_string()),
            right: ServerConfigurationMatcherOperand::String(r"^/admin/.*".to_string()),
            op: ServerConfigurationMatcherOperator::NotRegex,
        };
        resolver
            .insert_if_conditional(vec![not_regex_expr], Arc::clone(&config), 10)
            .expect("Valid regex");

        // Create variables with non-admin path (should match)
        let request = http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync());
        let mut ctx = make_test_context(request);
        ctx.variables
            .insert("path".to_string(), "/public/page".to_string());

        let mut path = ResolvedLocationPath::new();
        let configs = resolver.resolve_conditionals(&ctx, &mut path);

        // Should match (path does NOT match admin pattern)
        assert!(
            !configs.is_empty(),
            "NotRegex should match paths that don't match the pattern"
        );

        // Create variables with admin path (should not match)
        let request2 = http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync());
        let mut ctx2 = make_test_context(request2);
        ctx2.variables
            .insert("path".to_string(), "/admin/dashboard".to_string());

        let mut path2 = ResolvedLocationPath::new();
        let configs2 = resolver.resolve_conditionals(&ctx2, &mut path2);

        // Should not match (path DOES match admin pattern)
        assert!(
            configs2.is_empty(),
            "NotRegex should not match paths that match the pattern"
        );
    }

    #[test]
    fn test_compiled_matcher_expr_creation() {
        let expr = ServerConfigurationMatcherExpr {
            left: ServerConfigurationMatcherOperand::String("test".to_string()),
            right: ServerConfigurationMatcherOperand::String(r"^test$".to_string()),
            op: ServerConfigurationMatcherOperator::Regex,
        };

        let compiled = CompiledMatcherExpr::new(expr).expect("Should compile valid regex");
        assert!(
            compiled.compiled_regex.is_some(),
            "Regex should be compiled"
        );

        // Test with invalid regex pattern
        let invalid_expr = ServerConfigurationMatcherExpr {
            left: ServerConfigurationMatcherOperand::String("test".to_string()),
            right: ServerConfigurationMatcherOperand::String("(?P<invalid".to_string()), // Invalid group
            op: ServerConfigurationMatcherOperator::Regex,
        };

        let result = CompiledMatcherExpr::new(invalid_expr);
        assert!(result.is_err(), "Should fail on invalid regex pattern");
    }

    #[test]
    fn test_fancy_regex_features() {
        use ferron_core::config::{
            ServerConfigurationMatcherExpr, ServerConfigurationMatcherOperand,
            ServerConfigurationMatcherOperator,
        };

        let mut resolver = Stage2RadixResolver::new();
        let config = Arc::new(create_test_block());

        // Use fancy regex with lookahead: match paths containing "api" but not "admin"
        let lookahead_expr = ServerConfigurationMatcherExpr {
            left: ServerConfigurationMatcherOperand::Identifier("path".to_string()),
            right: ServerConfigurationMatcherOperand::String(r"^(?!.*admin).*api.*".to_string()),
            op: ServerConfigurationMatcherOperator::Regex,
        };
        resolver
            .insert_if_conditional(vec![lookahead_expr], config, 10)
            .expect("Valid regex");

        // Should match
        let request = http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync());
        let mut ctx = make_test_context(request);
        ctx.variables
            .insert("path".to_string(), "/api/users".to_string());

        let mut path = ResolvedLocationPath::new();
        let configs = resolver.resolve_conditionals(&ctx, &mut path);
        assert!(
            !configs.is_empty(),
            "Should match path with api but not admin"
        );

        // Should not match (contains admin)
        let request2 = http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync());
        let mut ctx2 = make_test_context(request2);
        ctx2.variables
            .insert("path".to_string(), "/admin/api/users".to_string());

        let mut path2 = ResolvedLocationPath::new();
        let configs2 = resolver.resolve_conditionals(&ctx2, &mut path2);
        assert!(
            configs2.is_empty(),
            "Should not match path containing admin"
        );
    }

    // ========================================================================
    // Configuration Chaining Tests - Multiple Branches Isolation
    // ========================================================================

    /// Test that configurations from unrelated IP branches don't leak
    #[test]
    fn test_branch_isolation_ip_level() {
        let mut resolver = ThreeStageResolver::new();

        // Setup two different IP branches with distinct configurations
        let mut hosts_ip1 = HostConfigs::new();
        let mut directives_ip1 = HashMap::new();
        directives_ip1.insert("ip1_directive".to_string(), vec![]);
        let host_block_ip1 = PreparedHostConfigurationBlock {
            directives: Arc::new(directives_ip1),
            matches: Vec::new(),
            error_config: Vec::new(),
        };
        hosts_ip1.insert(None, Arc::new(host_block_ip1));
        resolver
            .stage1()
            .register_ip("192.168.1.1".parse().unwrap(), hosts_ip1);

        let mut hosts_ip2 = HostConfigs::new();
        let mut directives_ip2 = HashMap::new();
        directives_ip2.insert("ip2_directive".to_string(), vec![]);
        let host_block_ip2 = PreparedHostConfigurationBlock {
            directives: Arc::new(directives_ip2),
            matches: Vec::new(),
            error_config: Vec::new(),
        };
        hosts_ip2.insert(None, Arc::new(host_block_ip2));
        resolver
            .stage1()
            .register_ip("192.168.1.2".parse().unwrap(), hosts_ip2);

        // Resolve for IP1 and IP2 - should remain isolated
        let (config1, path1) =
            resolver.resolve_stage1_layered("192.168.1.1".parse().unwrap(), None);
        let (config2, path2) =
            resolver.resolve_stage1_layered("192.168.1.2".parse().unwrap(), None);

        assert_eq!(path1.ip, Some("192.168.1.1".parse().unwrap()));
        assert_eq!(path2.ip, Some("192.168.1.2".parse().unwrap()));
        assert_eq!(config1.layers.len(), 1);
        assert_eq!(config2.layers.len(), 1);
        assert_ne!(path1.ip, path2.ip);
    }

    /// Test that configurations from unrelated hostname branches don't leak
    #[test]
    fn test_branch_isolation_hostname_level() {
        let mut resolver = Stage2RadixResolver::new();

        let config_example = Arc::new(create_test_block());
        let config_other = Arc::new(create_test_block());

        resolver.insert_host(vec!["com", "example"], Arc::clone(&config_example), 10);
        resolver.insert_host(vec!["com", "other"], Arc::clone(&config_other), 10);

        let mut path_example = ResolvedLocationPath::new();
        let configs_example = resolver.resolve_hostname("example.com", &mut path_example);
        assert_eq!(configs_example.len(), 1);
        assert_eq!(path_example.hostname_segments, vec!["example", "com"]);

        let mut path_other = ResolvedLocationPath::new();
        let configs_other = resolver.resolve_hostname("other.com", &mut path_other);
        assert_eq!(configs_other.len(), 1);
        assert_eq!(path_other.hostname_segments, vec!["other", "com"]);
        assert_ne!(path_example.hostname_segments, path_other.hostname_segments);
    }

    /// Test full three-stage chaining with multiple IP branches
    #[test]
    fn test_chained_resolution_ip_branch_isolation() {
        let mut resolver = ThreeStageResolver::new();

        // Setup IP branch 1: 192.168.1.1 -> example.com
        let mut hosts_ip1 = HostConfigs::new();
        let mut directives_ip1 = HashMap::new();
        directives_ip1.insert("ip1_layer".to_string(), vec![]);
        let host_block_ip1 = PreparedHostConfigurationBlock {
            directives: Arc::new(directives_ip1),
            matches: Vec::new(),
            error_config: Vec::new(),
        };
        hosts_ip1.insert(Some("example.com".to_string()), Arc::new(host_block_ip1));
        resolver
            .stage1()
            .register_ip("192.168.1.1".parse().unwrap(), hosts_ip1);

        // Setup IP branch 2: 192.168.1.2 -> other.com
        let mut hosts_ip2 = HostConfigs::new();
        let mut directives_ip2 = HashMap::new();
        directives_ip2.insert("ip2_layer".to_string(), vec![]);
        let host_block_ip2 = PreparedHostConfigurationBlock {
            directives: Arc::new(directives_ip2),
            matches: Vec::new(),
            error_config: Vec::new(),
        };
        hosts_ip2.insert(Some("other.com".to_string()), Arc::new(host_block_ip2));
        resolver
            .stage1()
            .register_ip("192.168.1.2".parse().unwrap(), hosts_ip2);

        // Setup Stage 2
        let mut directives_example = HashMap::new();
        directives_example.insert("example_layer".to_string(), vec![]);
        let example_block = PreparedHostConfigurationBlock {
            directives: Arc::new(directives_example),
            matches: Vec::new(),
            error_config: Vec::new(),
        };
        resolver
            .stage2()
            .insert_host(vec!["com", "example"], Arc::new(example_block), 10);

        let mut directives_other = HashMap::new();
        directives_other.insert("other_layer".to_string(), vec![]);
        let other_block = PreparedHostConfigurationBlock {
            directives: Arc::new(directives_other),
            matches: Vec::new(),
            error_config: Vec::new(),
        };
        resolver
            .stage2()
            .insert_host(vec!["com", "other"], Arc::new(other_block), 10);

        // Setup Stage 3
        resolver
            .stage3()
            .register_error(404, Arc::new(create_test_block()));
        resolver
            .stage3()
            .register_error(500, Arc::new(create_test_block()));

        let request = http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync());
        let ctx = make_test_context(request);
        let result1 = resolver.resolve("192.168.1.1".parse().unwrap(), "example.com", "/api", &ctx);
        let result2 =
            resolver.resolve("192.168.1.2".parse().unwrap(), "other.com", "/static", &ctx);

        assert!(result1.is_some());
        assert!(result2.is_some());

        let result1 = result1.unwrap();
        let result2 = result2.unwrap();

        assert_eq!(
            result1.location_path.ip,
            Some("192.168.1.1".parse().unwrap())
        );
        assert_eq!(
            result2.location_path.ip,
            Some("192.168.1.2".parse().unwrap())
        );
        assert_eq!(
            result1.location_path.hostname_segments,
            vec!["example", "com"]
        );
        assert_eq!(
            result2.location_path.hostname_segments,
            vec!["other", "com"]
        );
    }

    /// Test that wildcard branches don't leak into exact match branches
    #[test]
    fn test_branch_isolation_wildcard_vs_exact() {
        let mut resolver = Stage2RadixResolver::new();

        let wildcard_config = Arc::new(create_test_block());
        let exact_config = Arc::new(create_test_block());

        resolver.insert_host_wildcard(vec!["com", "example"], Arc::clone(&wildcard_config), 5);
        resolver.insert_host(vec!["com", "example", "www"], Arc::clone(&exact_config), 10);

        let mut path_exact = ResolvedLocationPath::new();
        let configs_exact = resolver.resolve_hostname("www.example.com", &mut path_exact);
        assert_eq!(configs_exact.len(), 2);

        let mut path_wildcard = ResolvedLocationPath::new();
        let configs_wildcard = resolver.resolve_hostname("sub.example.com", &mut path_wildcard);
        assert_eq!(configs_wildcard.len(), 1);

        let mut path_none = ResolvedLocationPath::new();
        let configs_none = resolver.resolve_hostname("other.com", &mut path_none);
        assert_eq!(configs_none.len(), 0);
    }

    /// Test conditional branches don't leak into each other
    #[test]
    fn test_branch_isolation_conditionals() {
        use ferron_core::config::{
            ServerConfigurationMatcherExpr, ServerConfigurationMatcherOperand,
            ServerConfigurationMatcherOperator,
        };

        let mut resolver = Stage2RadixResolver::new();

        let config_get = Arc::new(create_test_block());
        let config_post = Arc::new(create_test_block());

        let get_expr = ServerConfigurationMatcherExpr {
            left: ServerConfigurationMatcherOperand::Identifier("method".to_string()),
            right: ServerConfigurationMatcherOperand::String("GET".to_string()),
            op: ServerConfigurationMatcherOperator::Eq,
        };
        let post_expr = ServerConfigurationMatcherExpr {
            left: ServerConfigurationMatcherOperand::Identifier("method".to_string()),
            right: ServerConfigurationMatcherOperand::String("POST".to_string()),
            op: ServerConfigurationMatcherOperator::Eq,
        };

        resolver
            .insert_if_conditional(vec![get_expr], Arc::clone(&config_get), 10)
            .expect("Valid GET conditional");
        resolver
            .insert_if_conditional(vec![post_expr], Arc::clone(&config_post), 10)
            .expect("Valid POST conditional");

        let request_get = http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync());
        let mut ctx_get = make_test_context(request_get);
        ctx_get
            .variables
            .insert("method".to_string(), "GET".to_string());

        let mut path_get = ResolvedLocationPath::new();
        let configs_get = resolver.resolve_conditionals(&ctx_get, &mut path_get);
        assert_eq!(configs_get.len(), 1);

        let request_post = http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync());
        let mut ctx_post = make_test_context(request_post);
        ctx_post
            .variables
            .insert("method".to_string(), "POST".to_string());

        let mut path_post = ResolvedLocationPath::new();
        let configs_post = resolver.resolve_conditionals(&ctx_post, &mut path_post);
        assert_eq!(configs_post.len(), 1);

        let request_delete =
            http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync());
        let mut ctx_delete = make_test_context(request_delete);
        ctx_delete
            .variables
            .insert("method".to_string(), "DELETE".to_string());

        let mut path_delete = ResolvedLocationPath::new();
        let configs_delete = resolver.resolve_conditionals(&ctx_delete, &mut path_delete);
        assert_eq!(configs_delete.len(), 0);
    }

    /// Test error configuration branches don't leak
    #[test]
    fn test_branch_isolation_error_codes() {
        let mut resolver = Stage3ErrorResolver::new();

        let config_404 = Arc::new(create_test_block());
        let config_500 = Arc::new(create_test_block());
        let config_default = Arc::new(create_test_block());

        resolver.register_error(404, Arc::clone(&config_404));
        resolver.register_error(500, Arc::clone(&config_500));
        resolver.set_default(Arc::clone(&config_default));

        let (config_404_result, path_404) = resolver.resolve_layered(404, None);
        assert_eq!(path_404.error_key, Some(404));
        assert_eq!(config_404_result.layers.len(), 1);

        let (config_500_result, path_500) = resolver.resolve_layered(500, None);
        assert_eq!(path_500.error_key, Some(500));
        assert_eq!(config_500_result.layers.len(), 1);

        let (config_default_result, path_default) = resolver.resolve_layered(418, None);
        assert_eq!(path_default.error_key, Some(418));
        assert_eq!(config_default_result.layers.len(), 1);
    }

    /// Test complex scenario with multiple parallel branches at all stages
    #[test]
    fn test_complex_multi_branch_chaining() {
        let mut resolver = ThreeStageResolver::new();

        // Branch A: 10.0.0.1 -> api.com
        // Branch B: 10.0.0.2 -> other.com

        let mut hosts_a = HostConfigs::new();
        let mut directives_a = HashMap::new();
        directives_a.insert("branch_a_ip".to_string(), vec![]);
        hosts_a.insert(
            Some("api.com".to_string()),
            Arc::new(PreparedHostConfigurationBlock {
                directives: Arc::new(directives_a),
                matches: Vec::new(),
                error_config: Vec::new(),
            }),
        );
        resolver
            .stage1()
            .register_ip("10.0.0.1".parse().unwrap(), hosts_a);

        let mut hosts_b = HostConfigs::new();
        let mut directives_b = HashMap::new();
        directives_b.insert("branch_b_ip".to_string(), vec![]);
        hosts_b.insert(
            Some("other.com".to_string()),
            Arc::new(PreparedHostConfigurationBlock {
                directives: Arc::new(directives_b),
                matches: Vec::new(),
                error_config: Vec::new(),
            }),
        );
        resolver
            .stage1()
            .register_ip("10.0.0.2".parse().unwrap(), hosts_b);

        // Setup Stage 2
        let mut directives_api = HashMap::new();
        directives_api.insert("branch_a_hostname".to_string(), vec![]);
        resolver.stage2().insert_host(
            vec!["com", "api"],
            Arc::new(PreparedHostConfigurationBlock {
                directives: Arc::new(directives_api),
                matches: Vec::new(),
                error_config: Vec::new(),
            }),
            10,
        );

        let mut directives_other = HashMap::new();
        directives_other.insert("branch_b_hostname".to_string(), vec![]);
        resolver.stage2().insert_host(
            vec!["com", "other"],
            Arc::new(PreparedHostConfigurationBlock {
                directives: Arc::new(directives_other),
                matches: Vec::new(),
                error_config: Vec::new(),
            }),
            10,
        );

        // Setup Stage 3
        resolver
            .stage3()
            .register_error(400, Arc::new(create_test_block()));
        resolver
            .stage3()
            .register_error(500, Arc::new(create_test_block()));

        let request = http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync());
        let ctx = make_test_context(request);

        let result_a = resolver.resolve("10.0.0.1".parse().unwrap(), "api.com", "/v1/users", &ctx);
        let result_b = resolver.resolve("10.0.0.2".parse().unwrap(), "other.com", "/home", &ctx);

        assert!(result_a.is_some(), "Branch A should resolve");
        assert!(result_b.is_some(), "Branch B should resolve");

        let result_a = result_a.unwrap();
        let result_b = result_b.unwrap();

        assert_eq!(result_a.location_path.ip, Some("10.0.0.1".parse().unwrap()));
        assert_eq!(result_b.location_path.ip, Some("10.0.0.2".parse().unwrap()));
        assert_ne!(
            result_a.location_path.ip, result_b.location_path.ip,
            "Branches should be isolated"
        );
        assert_eq!(result_a.location_path.hostname_segments, vec!["api", "com"]);
        assert_eq!(
            result_b.location_path.hostname_segments,
            vec!["other", "com"]
        );
    }

    /// Test that layered configuration properly chains without cross-branch contamination
    #[test]
    fn test_layered_chaining_isolation() {
        let mut resolver = ThreeStageResolver::new();

        // Chain 1: IP1 -> host1
        // Chain 2: IP2 -> host2

        let mut hosts1 = HostConfigs::new();
        let mut directives1 = HashMap::new();
        directives1.insert("layer1_ip".to_string(), vec![]);
        hosts1.insert(
            Some("host1.com".to_string()),
            Arc::new(PreparedHostConfigurationBlock {
                directives: Arc::new(directives1),
                matches: Vec::new(),
                error_config: Vec::new(),
            }),
        );
        resolver
            .stage1()
            .register_ip("1.1.1.1".parse().unwrap(), hosts1);

        let mut hosts2 = HostConfigs::new();
        let mut directives2 = HashMap::new();
        directives2.insert("layer2_ip".to_string(), vec![]);
        hosts2.insert(
            Some("host2.com".to_string()),
            Arc::new(PreparedHostConfigurationBlock {
                directives: Arc::new(directives2),
                matches: Vec::new(),
                error_config: Vec::new(),
            }),
        );
        resolver
            .stage1()
            .register_ip("2.2.2.2".parse().unwrap(), hosts2);

        // Setup Stage 2
        let mut h1_directives = HashMap::new();
        h1_directives.insert("layer1_host".to_string(), vec![]);
        resolver.stage2().insert_host(
            vec!["com", "host1"],
            Arc::new(PreparedHostConfigurationBlock {
                directives: Arc::new(h1_directives),
                matches: Vec::new(),
                error_config: Vec::new(),
            }),
            10,
        );

        let mut h2_directives = HashMap::new();
        h2_directives.insert("layer2_host".to_string(), vec![]);
        resolver.stage2().insert_host(
            vec!["com", "host2"],
            Arc::new(PreparedHostConfigurationBlock {
                directives: Arc::new(h2_directives),
                matches: Vec::new(),
                error_config: Vec::new(),
            }),
            10,
        );

        // Setup Stage 3
        resolver
            .stage3()
            .register_error(404, Arc::new(create_test_block()));
        resolver
            .stage3()
            .register_error(500, Arc::new(create_test_block()));

        let request = http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync());
        let ctx = make_test_context(request);

        let result1 = resolver.resolve("1.1.1.1".parse().unwrap(), "host1.com", "/", &ctx);
        let result2 = resolver.resolve("2.2.2.2".parse().unwrap(), "host2.com", "/", &ctx);

        assert!(result1.is_some());
        assert!(result2.is_some());

        let result1 = result1.unwrap();
        let result2 = result2.unwrap();

        assert_eq!(result1.location_path.ip, Some("1.1.1.1".parse().unwrap()));
        assert_eq!(result2.location_path.ip, Some("2.2.2.2".parse().unwrap()));
        assert_eq!(
            result1.location_path.hostname_segments,
            vec!["host1", "com"]
        );
        assert_eq!(
            result2.location_path.hostname_segments,
            vec!["host2", "com"]
        );
    }

    /// Test IP1 -> Host1 -> Error1 vs IP2 -> Host1 -> Error2
    /// Verifies that the same hostname accessed from different IPs can have
    /// different configurations based on the IP source
    #[test]
    fn test_same_host_different_ip_error_chaining() {
        use ferron_core::config::{ServerConfigurationDirectiveEntry, ServerConfigurationValue};

        let mut resolver = ThreeStageResolver::new();

        // Both IPs serve the same hostname "shared.com" but with different configs
        // IP1 (192.168.1.1) -> shared.com -> "ip1" source
        // IP2 (192.168.1.2) -> shared.com -> "ip2" source

        // Setup Stage 1 - both IPs have the same hostname
        let mut hosts_ip1 = HostConfigs::new();
        let mut ip1_directives = HashMap::new();
        ip1_directives.insert(
            "ip_source".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String("ip1".to_string(), None)],
                children: None,
                span: None,
            }],
        );
        hosts_ip1.insert(
            Some("shared.com".to_string()),
            Arc::new(PreparedHostConfigurationBlock {
                directives: Arc::new(ip1_directives),
                matches: Vec::new(),
                error_config: Vec::new(),
            }),
        );
        resolver
            .stage1()
            .register_ip("192.168.1.1".parse().unwrap(), hosts_ip1);

        let mut hosts_ip2 = HostConfigs::new();
        let mut ip2_directives = HashMap::new();
        ip2_directives.insert(
            "ip_source".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String("ip2".to_string(), None)],
                children: None,
                span: None,
            }],
        );
        hosts_ip2.insert(
            Some("shared.com".to_string()),
            Arc::new(PreparedHostConfigurationBlock {
                directives: Arc::new(ip2_directives),
                matches: Vec::new(),
                error_config: Vec::new(),
            }),
        );
        resolver
            .stage1()
            .register_ip("192.168.1.2".parse().unwrap(), hosts_ip2);

        // Setup Stage 2 - same hostname for both
        let mut shared_directives = HashMap::new();
        shared_directives.insert(
            "hostname".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String("shared".to_string(), None)],
                children: None,
                span: None,
            }],
        );
        resolver.stage2().insert_host(
            vec!["com", "shared"],
            Arc::new(PreparedHostConfigurationBlock {
                directives: Arc::new(shared_directives),
                matches: Vec::new(),
                error_config: Vec::new(),
            }),
            10,
        );

        // Setup Stage 3 - global error config (shared across IPs)
        let mut error_404 = HashMap::new();
        error_404.insert(
            "error_source".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String(
                    "global_404".to_string(),
                    None,
                )],
                children: None,
                span: None,
            }],
        );
        resolver.stage3().register_error(
            404,
            Arc::new(PreparedHostConfigurationBlock {
                directives: Arc::new(error_404),
                matches: Vec::new(),
                error_config: Vec::new(),
            }),
        );

        let request = http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync());
        let ctx = make_test_context(request);

        // Resolve from IP1
        let result_ip1 = resolver.resolve("192.168.1.1".parse().unwrap(), "shared.com", "/", &ctx);

        // Resolve from IP2
        let result_ip2 = resolver.resolve("192.168.1.2".parse().unwrap(), "shared.com", "/", &ctx);

        assert!(result_ip1.is_some(), "IP1 should resolve");
        assert!(result_ip2.is_some(), "IP2 should resolve");

        let result_ip1 = result_ip1.unwrap();
        let result_ip2 = result_ip2.unwrap();

        // Verify IPs are correctly tracked
        assert_eq!(
            result_ip1.location_path.ip,
            Some("192.168.1.1".parse().unwrap())
        );
        assert_eq!(
            result_ip2.location_path.ip,
            Some("192.168.1.2".parse().unwrap())
        );

        // Verify both resolve the same hostname
        assert_eq!(
            result_ip1.location_path.hostname_segments,
            vec!["shared", "com"]
        );
        assert_eq!(
            result_ip2.location_path.hostname_segments,
            vec!["shared", "com"]
        );

        // Verify Stage 1 IP source directive is preserved and isolated
        let ip1_source = result_ip1.configuration.get_value("ip_source", true);
        let ip2_source = result_ip2.configuration.get_value("ip_source", true);

        if let Some(ServerConfigurationValue::String(val, _)) = ip1_source {
            assert_eq!(val, "ip1", "IP1 should have ip1 source");
        } else {
            panic!("IP1 should have ip_source directive");
        }

        if let Some(ServerConfigurationValue::String(val, _)) = ip2_source {
            assert_eq!(val, "ip2", "IP2 should have ip2 source");
        } else {
            panic!("IP2 should have ip_source directive");
        }

        // Verify hostname directive is present in both
        let hostname_val1 = result_ip1.configuration.get_value("hostname", true);
        let hostname_val2 = result_ip2.configuration.get_value("hostname", true);

        assert!(
            hostname_val1.is_some(),
            "IP1 should have hostname directive"
        );
        assert!(
            hostname_val2.is_some(),
            "IP2 should have hostname directive"
        );

        // Verify error directive is the same (global) for both
        let error_val1 = result_ip1.configuration.get_value("error_source", true);
        let error_val2 = result_ip2.configuration.get_value("error_source", true);

        if let Some(ServerConfigurationValue::String(val, _)) = error_val1 {
            assert_eq!(val, "global_404", "IP1 should have global error config");
        }
        if let Some(ServerConfigurationValue::String(val, _)) = error_val2 {
            assert_eq!(val, "global_404", "IP2 should have global error config");
        }
    }

    /// Test Stage 2 → Stage 3 scoped chaining with hostname-specific error configs
    /// Demonstrates that error configs can now be scoped to specific hostnames
    #[test]
    fn test_stage2_to_stage3_scoped_chaining() {
        use ferron_core::config::{ServerConfigurationDirectiveEntry, ServerConfigurationValue};

        let mut resolver = ThreeStageResolver::new();

        // Setup: Two hostnames with different error configs for the same error code
        // api.com -> 404 -> "api_custom_404"
        // web.com -> 404 -> "web_custom_404"

        // Stage 1: Both IPs have their respective hostnames
        let mut hosts = HostConfigs::new();

        let mut api_directives = HashMap::new();
        api_directives.insert(
            "host".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String("api".to_string(), None)],
                children: None,
                span: None,
            }],
        );
        hosts.insert(
            Some("api.com".to_string()),
            Arc::new(PreparedHostConfigurationBlock {
                directives: Arc::new(api_directives),
                matches: Vec::new(),
                error_config: Vec::new(),
            }),
        );

        let mut web_directives = HashMap::new();
        web_directives.insert(
            "host".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String("web".to_string(), None)],
                children: None,
                span: None,
            }],
        );
        hosts.insert(
            Some("web.com".to_string()),
            Arc::new(PreparedHostConfigurationBlock {
                directives: Arc::new(web_directives),
                matches: Vec::new(),
                error_config: Vec::new(),
            }),
        );

        resolver
            .stage1()
            .register_ip("192.168.1.1".parse().unwrap(), hosts);

        // Stage 2: Hostname-specific configs
        let mut api_stage2 = HashMap::new();
        api_stage2.insert(
            "stage2".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String(
                    "api_stage2".to_string(),
                    None,
                )],
                children: None,
                span: None,
            }],
        );
        resolver.stage2().insert_host(
            vec!["com", "api"],
            Arc::new(PreparedHostConfigurationBlock {
                directives: Arc::new(api_stage2),
                matches: Vec::new(),
                error_config: Vec::new(),
            }),
            10,
        );

        let mut web_stage2 = HashMap::new();
        web_stage2.insert(
            "stage2".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String(
                    "web_stage2".to_string(),
                    None,
                )],
                children: None,
                span: None,
            }],
        );
        resolver.stage2().insert_host(
            vec!["com", "web"],
            Arc::new(PreparedHostConfigurationBlock {
                directives: Arc::new(web_stage2),
                matches: Vec::new(),
                error_config: Vec::new(),
            }),
            10,
        );

        // Stage 3: HOSTNAME-SPECIFIC error configs (new feature!)
        let mut api_error = HashMap::new();
        api_error.insert(
            "error_type".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String(
                    "api_404".to_string(),
                    None,
                )],
                children: None,
                span: None,
            }],
        );
        resolver.stage3().register_hostname_error(
            "api.com",
            404,
            Arc::new(PreparedHostConfigurationBlock {
                directives: Arc::new(api_error),
                matches: Vec::new(),
                error_config: Vec::new(),
            }),
        );

        let mut web_error = HashMap::new();
        web_error.insert(
            "error_type".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String(
                    "web_404".to_string(),
                    None,
                )],
                children: None,
                span: None,
            }],
        );
        resolver.stage3().register_hostname_error(
            "web.com",
            404,
            Arc::new(PreparedHostConfigurationBlock {
                directives: Arc::new(web_error),
                matches: Vec::new(),
                error_config: Vec::new(),
            }),
        );

        let request = http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync());
        let ctx = make_test_context(request);

        // Use the new resolve_error_scoped method
        let result_api = resolver.resolve_error_scoped(
            "192.168.1.1".parse().unwrap(),
            "api.com",
            "/",
            404,
            &ctx,
        );

        let result_web = resolver.resolve_error_scoped(
            "192.168.1.1".parse().unwrap(),
            "web.com",
            "/",
            404,
            &ctx,
        );

        assert!(result_api.is_some(), "API should resolve");
        assert!(result_web.is_some(), "Web should resolve");

        let result_api = result_api.unwrap();
        let result_web = result_web.unwrap();

        // Verify Stage 1 + Stage 2 directives
        let api_host = result_api.configuration.get_value("host", true);
        let web_host = result_web.configuration.get_value("host", true);

        if let Some(ServerConfigurationValue::String(val, _)) = api_host {
            assert_eq!(val, "api");
        }
        if let Some(ServerConfigurationValue::String(val, _)) = web_host {
            assert_eq!(val, "web");
        }

        // Verify Stage 2 directives
        let api_s2 = result_api.configuration.get_value("stage2", true);
        let web_s2 = result_web.configuration.get_value("stage2", true);

        if let Some(ServerConfigurationValue::String(val, _)) = api_s2 {
            assert_eq!(val, "api_stage2");
        }
        if let Some(ServerConfigurationValue::String(val, _)) = web_s2 {
            assert_eq!(val, "web_stage2");
        }

        // Verify Stage 3 HOSTNAME-SPECIFIC error configs (this is the new feature!)
        let api_err = result_api.configuration.get_value("error_type", true);
        let web_err = result_web.configuration.get_value("error_type", true);

        if let Some(ServerConfigurationValue::String(val, _)) = api_err {
            assert_eq!(val, "api_404", "API should have hostname-specific error");
        }
        if let Some(ServerConfigurationValue::String(val, _)) = web_err {
            assert_eq!(val, "web_404", "Web should have hostname-specific error");
        }
    }

    #[test]
    fn test_error_config_with_conditionals() {
        use ferron_core::config::{
            ServerConfigurationDirectiveEntry, ServerConfigurationMatcherExpr,
            ServerConfigurationMatcherOperand, ServerConfigurationMatcherOperator,
            ServerConfigurationValue,
        };

        let mut resolver = Stage3ErrorResolver::new();

        // Create a base config for error handling
        let mut error_directives = std::collections::HashMap::new();
        error_directives.insert(
            "error_type".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String(
                    "conditional_404".to_string(),
                    Default::default(),
                )],
                children: None,
                span: None,
            }],
        );
        let error_config = Arc::new(PreparedHostConfigurationBlock {
            directives: Arc::new(error_directives),
            matches: Vec::new(),
            error_config: Vec::new(),
        });

        // Create a conditional expression: request.method == "GET"
        let conditionals = vec![ConditionalGroup {
            exprs: vec![ServerConfigurationMatcherExpr {
                left: ServerConfigurationMatcherOperand::Identifier("request.method".to_string()),
                right: ServerConfigurationMatcherOperand::String("GET".to_string()),
                op: ServerConfigurationMatcherOperator::Eq,
            }],
            negated: false,
        }];

        // Create a scope with conditionals
        let scope = ErrorConfigScope {
            ip: None,
            hostname: None,
            path: None,
            conditionals: conditionals.clone(),
            error_code: Some(404),
        };

        // Register the conditional error config
        resolver.register(scope, error_config);

        // Create test context with a GET request
        let get_request = http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync());
        let ctx_get = make_test_context(get_request);

        // Test that conditional matches for GET request
        let mut path_get = ResolvedLocationPath::new();
        let result_get = resolver.resolve_scoped(404, None, None, None, &ctx_get, &mut path_get);

        assert!(
            result_get.is_some(),
            "Should match conditional error config for GET request"
        );

        // Create test context with a POST request
        let mut post_request =
            http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync());
        *post_request.method_mut() = http::Method::POST;
        let ctx_post = make_test_context(post_request);

        // Test that conditional doesn't match for POST request
        let mut path_post = ResolvedLocationPath::new();
        let result_post = resolver.resolve_scoped(404, None, None, None, &ctx_post, &mut path_post);

        assert!(
            result_post.is_none(),
            "Should NOT match conditional error config for POST request"
        );
    }

    #[test]
    fn test_from_prepared_registers_conditionals_stage2() {
        use ferron_core::config::{
            ServerConfigurationDirectiveEntry, ServerConfigurationMatcherExpr,
            ServerConfigurationMatcherOperand, ServerConfigurationMatcherOperator,
            ServerConfigurationValue,
        };

        // Build a PreparedHostConfigurationBlock with an IfConditional matcher
        let mut cond_cfg = HashMap::new();
        cond_cfg.insert(
            "conditional_directive".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String(
                    "if_value".to_string(),
                    None,
                )],
                children: None,
                span: None,
            }],
        );

        let expr = ServerConfigurationMatcherExpr {
            left: ServerConfigurationMatcherOperand::Identifier("method".to_string()),
            right: ServerConfigurationMatcherOperand::String("GET".to_string()),
            op: ServerConfigurationMatcherOperator::Eq,
        };

        let host_block = PreparedHostConfigurationBlock {
            directives: Arc::new(HashMap::new()),
            matches: vec![PreparedHostConfigurationMatch {
                matcher: PreparedHostConfigurationMatcher::IfConditional(vec![expr]),
                config: Arc::new(PreparedHostConfigurationBlock {
                    directives: Arc::new(cond_cfg),
                    matches: Vec::new(),
                    error_config: Vec::new(),
                }),
            }],
            error_config: Vec::new(),
        };

        let mut hosts = HostConfigs::new();
        hosts.insert(Some("example.com".to_string()), Arc::new(host_block));

        let mut prepared: PreparedConfiguration = PreparedConfiguration::new();
        prepared.insert(None, hosts);

        let resolver = ThreeStageResolver::from_prepared(prepared);

        // Verify Stage 2 has the conditional registered by resolving it
        let request = http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync());
        let mut ctx = make_test_context(request);
        ctx.variables
            .insert("method".to_string(), "GET".to_string());

        let mut path = ResolvedLocationPath::new();
        let configs = resolver.stage2_ref().resolve_conditionals(&ctx, &mut path);
        assert!(
            !configs.is_empty(),
            "Stage 2 should have IfConditional registered from from_prepared"
        );
    }

    #[test]
    fn test_from_prepared_registers_location_error_configs_stage3() {
        use ferron_core::config::{ServerConfigurationDirectiveEntry, ServerConfigurationValue};

        // Build a PreparedHostConfigurationBlock with a Location matcher containing error config
        let mut error_directives = HashMap::new();
        error_directives.insert(
            "error_type".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String(
                    "location_404".to_string(),
                    None,
                )],
                children: None,
                span: None,
            }],
        );

        let location_block = PreparedHostConfigurationBlock {
            directives: Arc::new(error_directives),
            matches: Vec::new(),
            error_config: vec![PreparedHostConfigurationErrorConfig {
                error_code: Some(404),
                config: PreparedHostConfigurationBlock {
                    directives: Arc::new(HashMap::new()),
                    matches: Vec::new(),
                    error_config: Vec::new(),
                },
            }],
        };

        let host_block = PreparedHostConfigurationBlock {
            directives: Arc::new(HashMap::new()),
            matches: vec![PreparedHostConfigurationMatch {
                matcher: PreparedHostConfigurationMatcher::Location("/api".to_string()),
                config: Arc::new(location_block),
            }],
            error_config: Vec::new(),
        };

        let mut hosts = HostConfigs::new();
        hosts.insert(Some("example.com".to_string()), Arc::new(host_block));

        let mut prepared: PreparedConfiguration = PreparedConfiguration::new();
        prepared.insert(None, hosts);

        let resolver = ThreeStageResolver::from_prepared(prepared);

        // Verify Stage 3 has the location error config registered
        let request = http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync());
        let ctx = make_test_context(request);
        let mut path = ResolvedLocationPath::new();
        let result = resolver.stage3_ref().resolve_scoped(
            404,
            Some("example.com"),
            None,
            Some(&["/api".to_string()]),
            &ctx,
            &mut path,
        );
        assert!(
            result.is_some(),
            "Stage 3 should have location error config registered from from_prepared"
        );
    }

    #[test]
    fn test_from_prepared_nested_location_and_conditional_error_configs() {
        use ferron_core::config::{
            ServerConfigurationDirectiveEntry, ServerConfigurationMatcherExpr,
            ServerConfigurationMatcherOperand, ServerConfigurationMatcherOperator,
            ServerConfigurationValue,
        };

        // Build a nested structure:
        // host_block
        //   -> Location("/api")
        //        -> IfConditional(method == POST)
        //             -> error_config for 500
        let mut nested_error_directives = HashMap::new();
        nested_error_directives.insert(
            "nested_error".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String(
                    "nested_500".to_string(),
                    None,
                )],
                children: None,
                span: None,
            }],
        );

        let conditional_expr = ServerConfigurationMatcherExpr {
            left: ServerConfigurationMatcherOperand::Identifier("request.method".to_string()),
            right: ServerConfigurationMatcherOperand::String("POST".to_string()),
            op: ServerConfigurationMatcherOperator::Eq,
        };

        let nested_block = PreparedHostConfigurationBlock {
            directives: Arc::new(nested_error_directives),
            matches: Vec::new(),
            error_config: vec![PreparedHostConfigurationErrorConfig {
                error_code: Some(500),
                config: PreparedHostConfigurationBlock {
                    directives: Arc::new(HashMap::new()),
                    matches: Vec::new(),
                    error_config: Vec::new(),
                },
            }],
        };

        let if_matcher = PreparedHostConfigurationMatch {
            matcher: PreparedHostConfigurationMatcher::IfConditional(vec![conditional_expr]),
            config: Arc::new(nested_block),
        };

        let location_block = PreparedHostConfigurationBlock {
            directives: Arc::new(HashMap::new()),
            matches: vec![if_matcher],
            error_config: Vec::new(),
        };

        let host_block = PreparedHostConfigurationBlock {
            directives: Arc::new(HashMap::new()),
            matches: vec![PreparedHostConfigurationMatch {
                matcher: PreparedHostConfigurationMatcher::Location("/api".to_string()),
                config: Arc::new(location_block),
            }],
            error_config: Vec::new(),
        };

        let mut hosts = HostConfigs::new();
        hosts.insert(Some("example.com".to_string()), Arc::new(host_block));

        let mut prepared: PreparedConfiguration = PreparedConfiguration::new();
        prepared.insert(None, hosts);

        let resolver = ThreeStageResolver::from_prepared(prepared);

        // Verify Stage 3 has the nested conditional error config
        let mut post_request =
            http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync());
        *post_request.method_mut() = http::Method::POST;
        let ctx_post = make_test_context(post_request);

        let mut path = ResolvedLocationPath::new();
        let result = resolver.stage3_ref().resolve_scoped(
            500,
            Some("example.com"),
            None,
            Some(&["/api".to_string()]),
            &ctx_post,
            &mut path,
        );
        assert!(
            result.is_some(),
            "Stage 3 should have nested nested location + conditional error config"
        );
    }
}
