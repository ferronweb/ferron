//! 3-Stage Configuration Resolver
//!
//! This module provides a modular configuration resolution system with three independent stages:
//!
//! 1. **Stage 1** - IP address-based resolution (BTreeMap)
//! 2. **Stage 2** - Main resolution using radix tree (hostname segments, wildcards, path segments, conditionals)
//! 3. **Stage 3** - Error configuration resolution (HashMap)
//!
//! Each stage can be used independently or composed together via the main resolver.

use std::{
    collections::{BTreeMap, HashMap},
    net::IpAddr,
    sync::Arc,
};

use ferron_core::config::{
    layer::LayeredConfiguration, ServerConfigurationBlock, ServerConfigurationMatcherExpr,
    ServerConfigurationMatcherOperand, ServerConfigurationMatcherOperator,
};

use crate::config::prepare_host_config;

use super::prepare::{
    PreparedConfiguration, PreparedHostConfigurationBlock, PreparedHostConfigurationErrorConfig,
    PreparedHostConfigurationMatcher,
};

/// Variables that can be used in conditional matching
pub type ResolverVariables = HashMap<String, String>;

/// Represents a resolved location path through the configuration tree
#[derive(Debug, Clone, Default)]
pub struct ResolvedLocationPath {
    /// IP address filter (Stage 1)
    pub ip: Option<IpAddr>,
    /// Hostname segments from root to leaf (Stage 2)
    pub hostname_segments: Vec<String>,
    /// Path segments from root to leaf (Stage 2)
    pub path_segments: Vec<String>,
    /// Matched conditional expressions (Stage 2)
    pub conditionals: Vec<ServerConfigurationMatcherExpr>,
    /// Error configuration key (Stage 3)
    pub error_key: Option<u16>,
}

impl ResolvedLocationPath {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a human-readable representation of the path
    pub fn to_string(&self) -> String {
        let mut parts = Vec::new();

        if let Some(ip) = self.ip {
            parts.push(format!("ip={}", ip));
        }

        if !self.hostname_segments.is_empty() {
            parts.push(format!("host={}", self.hostname_segments.join(".")));
        }

        if !self.path_segments.is_empty() {
            parts.push(format!("path=/{}", self.path_segments.join("/")));
        }

        if !self.conditionals.is_empty() {
            parts.push(format!("conditionals={}", self.conditionals.len()));
        }

        if let Some(error_key) = &self.error_key {
            parts.push(format!("error={}", error_key));
        }

        if parts.is_empty() {
            "root".to_string()
        } else {
            parts.join(" > ")
        }
    }
}

/// Result of a configuration resolution
pub struct ResolutionResult {
    /// The layered configuration from all matched stages
    pub configuration: LayeredConfiguration,
    /// The resolved location path
    pub location_path: ResolvedLocationPath,
}

impl ResolutionResult {
    pub fn new(configuration: LayeredConfiguration, location_path: ResolvedLocationPath) -> Self {
        Self {
            configuration,
            location_path,
        }
    }
}

// ============================================================================
// Stage 1: IP Address-based Resolution
// ============================================================================

/// Stage 1 resolver: IP address-based configuration lookup
///
/// Uses a BTreeMap for ordered IP address lookups.
#[derive(Debug)]
pub struct Stage1IpResolver {
    /// Maps IP addresses to prepared host configurations
    ip_map: BTreeMap<IpAddr, HashMap<Option<String>, PreparedHostConfigurationBlock>>,
    /// Default configuration when no IP matches
    default: Option<HashMap<Option<String>, PreparedHostConfigurationBlock>>,
}

impl Stage1IpResolver {
    pub fn new() -> Self {
        Self {
            ip_map: BTreeMap::new(),
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
        let mut layered_config = base_config.unwrap_or_else(|| LayeredConfiguration::new());

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

// ============================================================================
// Stage 2: Custom Radix Tree-based Main Resolution
// ============================================================================

/// Key types for the radix tree
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum RadixKey {
    /// Hostname segment (e.g., "com", "example" from "example.com")
    HostSegment(String),
    /// Hostname wildcard (e.g., "*" from "*.example.com")
    HostWildcard,
    /// Path segment (e.g., "admin" from "/admin/users")
    PathSegment(String),
}

/// Node data stored in the radix tree
#[derive(Debug, Clone)]
pub struct RadixNodeData {
    /// Configuration block at this node
    pub config: Option<Arc<PreparedHostConfigurationBlock>>,
    /// Whether this node is a terminal (exact match) node
    pub is_terminal: bool,
    /// Priority for matching (higher = more specific)
    pub priority: u32,
}

impl Default for RadixNodeData {
    fn default() -> Self {
        Self {
            config: None,
            is_terminal: false,
            priority: 0,
        }
    }
}

/// A node in the custom radix tree
#[derive(Debug, Clone)]
struct RadixNode {
    key: Option<RadixKey>,
    data: RadixNodeData,
    children: BTreeMap<String, RadixNode>,
    wildcard_child: Option<Box<RadixNode>>,
}

impl RadixNode {
    fn new(key: Option<RadixKey>) -> Self {
        Self {
            key,
            data: RadixNodeData::default(),
            children: BTreeMap::new(),
            wildcard_child: None,
        }
    }
}

/// Stage 2 resolver: Custom radix tree-based hostname and path resolution
///
/// The radix tree structure allows for efficient longest-prefix matching
/// of hostname segments and path segments.
#[derive(Debug, Clone)]
pub struct Stage2RadixResolver {
    /// The radix tree for hostname matching
    host_tree: RadixNode,
    /// Conditional configurations (IfConditional)
    if_conditionals: Vec<(
        Vec<ServerConfigurationMatcherExpr>,
        Arc<PreparedHostConfigurationBlock>,
        u32,
    )>,
    /// Conditional configurations (IfNotConditional)
    if_not_conditionals: Vec<(
        Vec<ServerConfigurationMatcherExpr>,
        Arc<PreparedHostConfigurationBlock>,
        u32,
    )>,
}

impl Stage2RadixResolver {
    pub fn new() -> Self {
        Self {
            host_tree: RadixNode::new(None),
            if_conditionals: Vec::new(),
            if_not_conditionals: Vec::new(),
        }
    }

    /// Insert a configuration into the host tree
    ///
    /// # Arguments
    /// * `hostname_segments` - Segments in root-to-leaf order (e.g., ["com", "example"] for "example.com")
    /// * `config` - Configuration block to associate
    /// * `priority` - Match priority (higher = more specific)
    pub fn insert_host(
        &mut self,
        hostname_segments: Vec<&str>,
        config: Arc<PreparedHostConfigurationBlock>,
        priority: u32,
    ) {
        let mut current = &mut self.host_tree;

        for segment in hostname_segments {
            let key = segment.to_string();
            current = current.children.entry(key).or_insert_with(|| {
                RadixNode::new(Some(RadixKey::HostSegment(segment.to_string())))
            });
        }

        current.data = RadixNodeData {
            config: Some(config),
            is_terminal: true,
            priority,
        };
    }

    /// Insert a wildcard host configuration (e.g., "*.example.com")
    pub fn insert_host_wildcard(
        &mut self,
        base_segments: Vec<&str>,
        config: Arc<PreparedHostConfigurationBlock>,
        priority: u32,
    ) {
        let mut current = &mut self.host_tree;

        // First insert the base segments (root-to-leaf order)
        for segment in base_segments {
            let key = segment.to_string();
            current = current.children.entry(key).or_insert_with(|| {
                RadixNode::new(Some(RadixKey::HostSegment(segment.to_string())))
            });
        }

        // Add wildcard child
        current.wildcard_child = Some(Box::new(RadixNode {
            key: Some(RadixKey::HostWildcard),
            data: RadixNodeData {
                config: Some(config),
                is_terminal: true,
                priority,
            },
            children: BTreeMap::new(),
            wildcard_child: None,
        }));
    }

    /// Insert a conditional configuration (if directive)
    pub fn insert_if_conditional(
        &mut self,
        exprs: Vec<ServerConfigurationMatcherExpr>,
        config: Arc<PreparedHostConfigurationBlock>,
        priority: u32,
    ) {
        self.if_conditionals.push((exprs, config, priority));
    }

    /// Insert a negative conditional configuration (if_not directive)
    pub fn insert_if_not_conditional(
        &mut self,
        exprs: Vec<ServerConfigurationMatcherExpr>,
        config: Arc<PreparedHostConfigurationBlock>,
        priority: u32,
    ) {
        self.if_not_conditionals.push((exprs, config, priority));
    }

    /// Resolve hostname and return matching configurations
    ///
    /// Returns all matching configurations from most specific to least specific
    pub fn resolve_hostname(
        &self,
        hostname: &str,
        location_path: &mut ResolvedLocationPath,
    ) -> Vec<Arc<PreparedHostConfigurationBlock>> {
        // Split hostname and reverse to get root-to-leaf order (com, example, ...)
        let segments: Vec<&str> = hostname.split('.').rev().collect();
        let mut configs = Vec::new();
        let mut current_path = Vec::new();
        let mut result_paths = Vec::new();

        // Traverse the tree with segments in root-to-leaf order
        self.collect_hostname_matches(
            &self.host_tree,
            &segments,
            0,
            &mut configs,
            &mut current_path,
            &mut result_paths,
        );

        // Sort configs by priority (descending) and extract
        configs.sort_by(|a, b| b.0.cmp(&a.0));

        // Use the longest matched path (most specific match)
        if let Some(longest_path) = result_paths.into_iter().max_by_key(|p| p.len()) {
            // Reverse to get leaf-to-root order (example, com)
            location_path.hostname_segments = longest_path.into_iter().rev().collect();
        }

        configs.into_iter().map(|(_, c)| c).collect()
    }

    fn collect_hostname_matches(
        &self,
        node: &RadixNode,
        segments: &[&str],
        depth: usize,
        configs: &mut Vec<(u32, Arc<PreparedHostConfigurationBlock>)>,
        current_path: &mut Vec<String>,
        result_paths: &mut Vec<Vec<String>>,
    ) {
        if depth >= segments.len() {
            // We've consumed all segments, check if current node has a config
            if node.data.is_terminal {
                if let Some(ref config) = node.data.config {
                    configs.push((node.data.priority, Arc::clone(config)));
                    result_paths.push(current_path.clone());
                }
            }

            // Also check for wildcard at this level
            if let Some(wildcard) = &node.wildcard_child {
                if wildcard.data.is_terminal {
                    if let Some(ref config) = wildcard.data.config {
                        configs.push((wildcard.data.priority, Arc::clone(config)));
                        result_paths.push(current_path.clone());
                    }
                }
            }

            return;
        }

        let current_segment = segments[depth];

        // Try exact match
        if let Some(child) = node.children.get(current_segment) {
            current_path.push(current_segment.to_string());
            self.collect_hostname_matches(
                child,
                segments,
                depth + 1,
                configs,
                current_path,
                result_paths,
            );
            current_path.pop();
        }

        // Try wildcard match (matches any remaining segments)
        if let Some(wildcard) = &node.wildcard_child {
            if wildcard.data.is_terminal {
                if let Some(ref _config) = wildcard.data.config {
                    current_path.push("*".to_string());
                    self.collect_hostname_matches(
                        wildcard.as_ref(),
                        segments,
                        depth + 1,
                        configs,
                        current_path,
                        result_paths,
                    );
                    current_path.pop();
                }
            }
        }

        // Also check if current node is terminal (partial match)
        if node.data.is_terminal {
            if let Some(ref config) = node.data.config {
                configs.push((node.data.priority, Arc::clone(config)));
                result_paths.push(current_path.clone());
            }
        }
    }

    /// Resolve location path and return matching configurations
    ///
    /// This performs a prefix-based search through the location matches
    pub fn resolve_location(
        &self,
        path: &str,
        base_config: &PreparedHostConfigurationBlock,
        location_path: &mut ResolvedLocationPath,
    ) -> Vec<Arc<PreparedHostConfigurationBlock>> {
        let mut configs = Vec::new();

        // First, add the base configuration
        configs.push((0u32, Arc::new(base_config.clone())));

        // Find matching location directives
        for location_match in &base_config.matches {
            if let PreparedHostConfigurationMatcher::Location(location_path_str) =
                &location_match.matcher
            {
                if self.location_matches(path, location_path_str) {
                    // Calculate priority based on specificity (longer path = more specific)
                    let priority = location_path_str.len() as u32;
                    configs.push((priority, Arc::new(location_match.config.clone())));

                    // Update location path
                    location_path.path_segments = location_path_str
                        .trim_start_matches('/')
                        .split('/')
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string())
                        .collect();
                }
            }
        }

        // Sort by priority (descending)
        configs.sort_by(|a, b| b.0.cmp(&a.0));

        configs.into_iter().map(|(_, c)| c).collect()
    }

    /// Check if a path matches a location pattern
    ///
    /// Supports:
    /// - Exact match: "/api" matches "/api"
    /// - Prefix match: "/api/" matches "/api/users"
    /// - Regex-like patterns could be added later
    fn location_matches(&self, path: &str, pattern: &str) -> bool {
        // Exact match
        if path == pattern {
            return true;
        }

        // Prefix match (pattern with trailing slash)
        if pattern.ends_with('/') && path.starts_with(pattern) {
            return true;
        }

        // Prefix match (pattern without trailing slash, path has more segments)
        if !pattern.ends_with('/') && path.starts_with(&format!("{}/", pattern)) {
            return true;
        }

        false
    }

    /// Resolve conditionals with given variables
    pub fn resolve_conditionals(
        &self,
        variables: &ResolverVariables,
        location_path: &mut ResolvedLocationPath,
    ) -> Vec<Arc<PreparedHostConfigurationBlock>> {
        let mut configs = Vec::new();

        // Check IfConditional (if directive)
        for (exprs, config, priority) in &self.if_conditionals {
            if self.evaluate_conditions(exprs, variables) {
                configs.push((*priority, Arc::clone(config)));
                location_path.conditionals.extend(exprs.clone());
            }
        }

        // Check IfNotConditional (if_not directive)
        for (exprs, config, priority) in &self.if_not_conditionals {
            if !self.evaluate_conditions(exprs, variables) {
                configs.push((*priority, Arc::clone(config)));
                // For if_not, we still track the conditionals that were NOT matched
                location_path.conditionals.extend(exprs.clone());
            }
        }

        configs.sort_by(|a, b| b.0.cmp(&a.0));
        configs.into_iter().map(|(_, c)| c).collect()
    }

    /// Evaluate conditional expressions with given variables
    fn evaluate_conditions(
        &self,
        exprs: &[ServerConfigurationMatcherExpr],
        variables: &ResolverVariables,
    ) -> bool {
        // All expressions must match (AND logic)
        exprs
            .iter()
            .all(|expr| self.evaluate_condition(expr, variables))
    }

    /// Evaluate a single conditional expression with given variables
    fn evaluate_condition(
        &self,
        expr: &ServerConfigurationMatcherExpr,
        variables: &ResolverVariables,
    ) -> bool {
        let left_val = self.get_operand_value(&expr.left, variables);
        let right_val = self.get_operand_value(&expr.right, variables);

        match &expr.op {
            ServerConfigurationMatcherOperator::Eq => left_val == right_val,
            ServerConfigurationMatcherOperator::NotEq => left_val != right_val,
            ServerConfigurationMatcherOperator::Regex => {
                // TODO: use `fancy-regex`
                if let (Some(l), Some(r)) = (left_val, right_val) {
                    l.contains(&r)
                } else {
                    false
                }
            }
            ServerConfigurationMatcherOperator::NotRegex => {
                // TODO: use `fancy-regex`
                if let (Some(l), Some(r)) = (left_val, right_val) {
                    !l.contains(&r)
                } else {
                    false
                }
            }
            ServerConfigurationMatcherOperator::In => {
                // TODO: support Accept-Language style lists
                if let (Some(l), Some(r)) = (left_val, right_val) {
                    r.split(',').any(|item| item.trim() == l)
                } else {
                    false
                }
            }
        }
    }

    /// Get the string value of an operand
    fn get_operand_value(
        &self,
        operand: &ServerConfigurationMatcherOperand,
        variables: &ResolverVariables,
    ) -> Option<String> {
        match operand {
            ServerConfigurationMatcherOperand::Identifier(name) => {
                // TODO: obtain variables from HTTP request data
                variables.get(name).cloned().or_else(|| Some(name.clone()))
            }
            ServerConfigurationMatcherOperand::String(s) => Some(s.clone()),
            ServerConfigurationMatcherOperand::Integer(n) => Some(n.to_string()),
            ServerConfigurationMatcherOperand::Float(f) => Some(f.to_string()),
        }
    }

    /// Full Stage 2 resolution combining hostname, location, and conditionals
    ///
    /// # Arguments
    /// * `hostname` - Request hostname to resolve
    /// * `path` - Request path to resolve
    /// * `base_config` - The base prepared host configuration block
    /// * `variables` - Variables for conditional evaluation
    /// * `layered_config` - Optional base layered configuration to add layers to
    pub fn resolve(
        &self,
        hostname: Option<&str>,
        path: &str,
        base_config: &PreparedHostConfigurationBlock,
        variables: &ResolverVariables,
        layered_config: Option<LayeredConfiguration>,
    ) -> (LayeredConfiguration, ResolvedLocationPath) {
        let mut location_path = ResolvedLocationPath::new();
        let mut layered_config = layered_config.unwrap_or_else(|| LayeredConfiguration::new());

        // Resolve hostname
        if let Some(host) = hostname {
            for config in self.resolve_hostname(host, &mut location_path) {
                // Convert PreparedHostConfigurationBlock to ServerConfigurationBlock
                // Clone the Arc (cheap - just increments ref count, no HashMap clone)
                let block = ServerConfigurationBlock {
                    directives: Arc::clone(&config.directives),
                    matchers: HashMap::new(),
                    span: None,
                };
                layered_config.add_layer(Arc::new(block));
            }
        }

        // Resolve location paths
        for config in self.resolve_location(path, base_config, &mut location_path) {
            let block = ServerConfigurationBlock {
                directives: Arc::clone(&config.directives),
                matchers: HashMap::new(),
                span: None,
            };
            layered_config.add_layer(Arc::new(block));
        }

        // Resolve conditionals
        for config in self.resolve_conditionals(variables, &mut location_path) {
            let block = ServerConfigurationBlock {
                directives: Arc::clone(&config.directives),
                matchers: HashMap::new(),
                span: None,
            };
            layered_config.add_layer(Arc::new(block));
        }

        (layered_config, location_path)
    }
}

impl Default for Stage2RadixResolver {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Stage 3: Error Configuration Resolution
// ============================================================================

/// Stage 3 resolver: Error configuration lookup
///
/// Uses a HashMap for O(1) error code lookups.
#[derive(Debug, Clone)]
pub struct Stage3ErrorResolver {
    /// Maps error codes to configuration blocks
    error_map: HashMap<u16, Arc<PreparedHostConfigurationBlock>>,
    /// Default error configuration (no specific code)
    default: Option<Arc<PreparedHostConfigurationBlock>>,
}

impl Stage3ErrorResolver {
    pub fn new() -> Self {
        Self {
            error_map: HashMap::new(),
            default: None,
        }
    }

    /// Register an error configuration
    pub fn register_error(&mut self, code: u16, config: Arc<PreparedHostConfigurationBlock>) {
        self.error_map.insert(code, config);
    }

    /// Set the default error configuration
    pub fn set_default(&mut self, config: Arc<PreparedHostConfigurationBlock>) {
        self.default = Some(config);
    }

    /// Resolve error configuration by code
    pub fn resolve(
        &self,
        error_code: u16,
        location_path: &mut ResolvedLocationPath,
    ) -> Option<Arc<PreparedHostConfigurationBlock>> {
        location_path.error_key = Some(error_code);

        self.error_map
            .get(&error_code)
            .cloned()
            .or_else(|| self.default.clone())
    }

    /// Resolve error configuration and create a layered configuration
    ///
    /// # Arguments
    /// * `error_code` - Error code to resolve
    /// * `base_config` - Optional base layered configuration to add layers to
    pub fn resolve_layered(
        &self,
        error_code: u16,
        base_config: Option<LayeredConfiguration>,
    ) -> (LayeredConfiguration, ResolvedLocationPath) {
        let mut location_path = ResolvedLocationPath::new();
        let mut layered_config = base_config.unwrap_or_else(|| LayeredConfiguration::new());

        if let Some(config) = self.resolve(error_code, &mut location_path) {
            // Clone the Arc (cheap - just increments ref count)
            let block = ServerConfigurationBlock {
                directives: Arc::clone(&config.directives),
                matchers: HashMap::new(),
                span: None,
            };
            layered_config.add_layer(Arc::new(block));
        }

        (layered_config, location_path)
    }

    /// Build from PreparedHostConfigurationBlock error configs
    pub fn from_error_configs(error_configs: &[PreparedHostConfigurationErrorConfig]) -> Self {
        let mut resolver = Self::new();

        for error_config in error_configs {
            let config = Arc::new(error_config.config.clone());
            if let Some(code) = error_config.error_code {
                resolver.register_error(code, config);
            } else {
                resolver.set_default(config);
            }
        }

        resolver
    }
}

impl Default for Stage3ErrorResolver {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Main 3-Stage Resolver
// ============================================================================

/// Combines all three resolver stages into a unified configuration resolver
#[derive(Debug)]
pub struct ThreeStageResolver {
    stage1_ip: Stage1IpResolver,
    stage2_radix: Stage2RadixResolver,
    stage3_error: Stage3ErrorResolver,
}

impl ThreeStageResolver {
    pub fn new() -> Self {
        Self {
            stage1_ip: Stage1IpResolver::new(),
            stage2_radix: Stage2RadixResolver::new(),
            stage3_error: Stage3ErrorResolver::new(),
        }
    }

    /// Create a resolver from prepared configuration
    pub fn from_prepared(prepared: &PreparedConfiguration) -> Self {
        let mut resolver = Self::new();

        for (ip_opt, hosts) in prepared {
            if let Some(ip) = ip_opt {
                resolver.stage1_ip.register_ip(*ip, hosts.clone());
            } else {
                resolver.stage1_ip.set_default(hosts.clone());
            }
        }

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
    /// * `variables` - Variables for conditional evaluation in Stage 2
    pub fn resolve(
        &self,
        ip: IpAddr,
        hostname: &str,
        path: &str,
        variables: &ResolverVariables,
    ) -> Option<ResolutionResult> {
        let mut location_path = ResolvedLocationPath::new();

        // Stage 1: IP-based resolution
        let host_configs = self.stage1_ip.resolve(ip, &mut location_path)?;

        // Get the host-specific configuration
        let host_config = host_configs
            .get(&Some(hostname.to_string()))
            .or_else(|| host_configs.get(&None))?;

        // Stage 2: Hostname, path, and conditional resolution (passing Stage 1's config)
        let (stage2_config, stage2_path) =
            self.stage2_radix
                .resolve(Some(hostname), path, host_config, variables, None);

        // Merge Stage 2 results
        let mut layered_config = LayeredConfiguration::new();
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
    pub fn resolve_stage1(
        &self,
        ip: IpAddr,
    ) -> Option<&HashMap<Option<String>, PreparedHostConfigurationBlock>> {
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
    /// * `base_config` - The base prepared host configuration block
    /// * `variables` - Variables for conditional evaluation
    pub fn resolve_stage2(
        &self,
        hostname: Option<&str>,
        path: &str,
        base_config: &PreparedHostConfigurationBlock,
        variables: &ResolverVariables,
    ) -> (LayeredConfiguration, ResolvedLocationPath) {
        self.stage2_radix
            .resolve(hostname, path, base_config, variables, None)
    }

    /// Resolve only through Stage 2 (hostname/path/conditionals) with base layered config
    ///
    /// # Arguments
    /// * `hostname` - Request hostname to resolve
    /// * `path` - Request path to resolve
    /// * `base_config` - The base prepared host configuration block
    /// * `variables` - Variables for conditional evaluation
    /// * `layered_config` - Optional base layered configuration to add layers to
    pub fn resolve_stage2_layered(
        &self,
        hostname: Option<&str>,
        path: &str,
        base_config: &PreparedHostConfigurationBlock,
        variables: &ResolverVariables,
        layered_config: Option<LayeredConfiguration>,
    ) -> (LayeredConfiguration, ResolvedLocationPath) {
        self.stage2_radix
            .resolve(hostname, path, base_config, variables, layered_config)
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
}

impl Default for ThreeStageResolver {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

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
    fn test_stage2_hostname_resolution() {
        let mut resolver = Stage2RadixResolver::new();

        let config = Arc::new(create_test_block());

        // Insert example.com configuration
        resolver.insert_host(vec!["com", "example"], Arc::clone(&config), 10);

        let mut path = ResolvedLocationPath::new();
        let configs = resolver.resolve_hostname("example.com", &mut path);

        assert!(!configs.is_empty());
        assert_eq!(path.hostname_segments, vec!["example", "com"]);
    }

    #[test]
    fn test_stage2_wildcard_resolution() {
        let mut resolver = Stage2RadixResolver::new();

        let config = Arc::new(create_test_block());

        // Insert *.example.com wildcard configuration
        resolver.insert_host_wildcard(vec!["com", "example"], Arc::clone(&config), 5);

        let mut path = ResolvedLocationPath::new();
        let configs = resolver.resolve_hostname("sub.example.com", &mut path);

        assert!(!configs.is_empty());
    }

    #[test]
    fn test_stage3_error_resolver() {
        let mut resolver = Stage3ErrorResolver::new();

        let config = Arc::new(create_test_block());
        resolver.register_error(404, config);

        let mut path = ResolvedLocationPath::new();
        let result = resolver.resolve(404, &mut path);

        assert!(result.is_some());
        assert_eq!(path.error_key, Some(404));
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

    #[test]
    fn test_stage2_layered_resolution() {
        let mut resolver = Stage2RadixResolver::new();

        let config = Arc::new(create_test_block());
        resolver.insert_host(vec!["com", "example"], Arc::clone(&config), 10);

        let base_block = create_test_block();
        let (layered_config, path) = resolver.resolve(
            Some("example.com"),
            "/api",
            &base_block,
            &HashMap::new(),
            None,
        );

        assert!(!path.hostname_segments.is_empty());
        assert!(layered_config.layers.len() >= 1);
    }

    #[test]
    fn test_stage3_layered_resolution() {
        let mut resolver = Stage3ErrorResolver::new();

        let config = Arc::new(create_test_block());
        resolver.register_error(404, config);

        let (layered_config, path) = resolver.resolve_layered(404, None);

        assert_eq!(path.error_key, Some(404));
        assert_eq!(layered_config.layers.len(), 1);
    }

    #[test]
    fn test_chained_layered_resolution() {
        let mut resolver = ThreeStageResolver::new();

        // Setup Stage 1
        let mut hosts = HashMap::new();
        let mut directives1 = HashMap::new();
        directives1.insert("stage1_directive".to_string(), vec![]);
        let host_block = PreparedHostConfigurationBlock {
            directives: Arc::new(directives1),
            matches: Vec::new(),
            error_config: Vec::new(),
        };
        hosts.insert(Some("example.com".to_string()), host_block);
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
            .get(&Some("example.com".to_string()))
            .unwrap();

        let (config2, _) = resolver.resolve_stage2_layered(
            Some("example.com"),
            "/api",
            host_block,
            &HashMap::new(),
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
        let mut hosts = HashMap::new();
        let host_block = PreparedHostConfigurationBlock {
            directives: Arc::new(HashMap::new()),
            matches: Vec::new(),
            error_config: Vec::new(),
        };
        hosts.insert(Some("example.com".to_string()), host_block);

        resolver
            .stage1()
            .register_ip("127.0.0.1".parse().unwrap(), hosts);

        // Full resolution
        let result = resolver.resolve(
            "127.0.0.1".parse().unwrap(),
            "example.com",
            "/api/test",
            &HashMap::new(),
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
        resolver.insert_if_conditional(vec![expr], config, 10);

        let mut variables = HashMap::new();
        variables.insert("method".to_string(), "GET".to_string());

        let mut path = ResolvedLocationPath::new();
        let configs = resolver.resolve_conditionals(&variables, &mut path);

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

        let prepared = super::super::prepare::prepare_host_config(port).unwrap();
        let resolver = ThreeStageResolver::from_prepared(&prepared);

        assert!(resolver.resolve_stage1(ip).is_some());
    }
}
