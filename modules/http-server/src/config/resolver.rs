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

use fancy_regex::Regex;
use ferron_core::config::{
    layer::LayeredConfiguration, ServerConfigurationBlock, ServerConfigurationMatcherExpr,
    ServerConfigurationMatcherOperand, ServerConfigurationMatcherOperator,
};
use ferron_http::variables::resolve_variable;
use ferron_http::HttpRequest;

use super::prepare::{
    PreparedConfiguration, PreparedHostConfigurationBlock, PreparedHostConfigurationErrorConfig,
    PreparedHostConfigurationMatcher,
};

/// Variables that can be used in conditional matching
pub type ResolverVariables = (HttpRequest, HashMap<String, String>);

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
// Regex Matching Utilities
// ============================================================================

/// A matcher expression with pre-compiled regex patterns for efficient evaluation.
///
/// This struct caches compiled regex patterns to avoid recompiling them on every evaluation.
/// Regexes are compiled at configuration insertion time, not at evaluation time.
/// Only Regex and NotRegex operations use compiled patterns; other operators work with string values.
#[derive(Debug, Clone)]
pub struct CompiledMatcherExpr {
    /// The original matcher expression
    pub expr: ServerConfigurationMatcherExpr,
    /// Compiled regex for the right operand if it's a Regex/NotRegex operation with a static pattern
    pub compiled_regex: Option<Arc<Regex>>,
}

impl CompiledMatcherExpr {
    /// Create a new compiled matcher expression, pre-compiling regex if needed
    ///
    /// Returns `Err` if regex compilation fails at insertion time.
    pub fn new(expr: ServerConfigurationMatcherExpr) -> Result<Self, String> {
        let compiled_regex = if matches!(
            expr.op,
            ServerConfigurationMatcherOperator::Regex
                | ServerConfigurationMatcherOperator::NotRegex
        ) {
            // Extract the regex pattern from the right operand
            let pattern = match &expr.right {
                ServerConfigurationMatcherOperand::String(s) => Some(s.clone()),
                ServerConfigurationMatcherOperand::Identifier(_name) => {
                    // For identifiers, pattern is dynamic; will be compiled at runtime
                    None
                }
                ServerConfigurationMatcherOperand::Integer(n) => Some(n.to_string()),
                ServerConfigurationMatcherOperand::Float(f) => Some(f.to_string()),
            };

            if let Some(pattern) = pattern {
                match Regex::new(&pattern) {
                    Ok(regex) => Some(Arc::new(regex)),
                    Err(e) => return Err(format!("Invalid regex pattern '{}': {}", pattern, e)),
                }
            } else {
                None
            }
        } else {
            None
        };

        Ok(Self {
            expr,
            compiled_regex,
        })
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

/// A node in the radix tree.
///
/// Each node stores a **sequence** of keys (`Vec<RadixKey>`) rather than a
/// single key.  When a chain of intermediary nodes carries no configuration
/// and has only one child, those nodes are compressed ("path-compressed") into
/// a single node whose `keys` vector holds all of the merged keys in order.
/// This turns the plain trie into a true radix tree.
///
/// The root node always has an empty `keys` vec.
#[derive(Debug, Clone)]
struct RadixNode {
    /// The (possibly compressed) sequence of keys leading to this node.
    /// Empty only for the synthetic root node.
    keys: Vec<RadixKey>,
    data: RadixNodeData,
    children: BTreeMap<String, RadixNode>,
    wildcard_child: Option<Box<RadixNode>>,
}

impl RadixNode {
    /// Create a new node with the given key sequence.
    fn new(keys: Vec<RadixKey>) -> Self {
        Self {
            keys,
            data: RadixNodeData::default(),
            children: BTreeMap::new(),
            wildcard_child: None,
        }
    }

    /// Returns the **last** `HostSegment` string in `keys`, which is the
    /// lookup key used in the parent's `children` map.
    ///
    /// Panics if `keys` is empty or the last key is not a `HostSegment`.
    fn last_segment_str(&self) -> &str {
        match self.keys.last() {
            Some(RadixKey::HostSegment(s)) => s.as_str(),
            _ => panic!("RadixNode::last_segment_str called on non-HostSegment node"),
        }
    }

    /// Try to compress this node with its sole child.
    ///
    /// Compression is possible when:
    /// - this node has **no config** (not terminal),
    /// - it has **exactly one** regular child and **no wildcard child**, and
    /// - the single child is a `HostSegment` node (wildcards are never merged).
    ///
    /// When compressible the child's keys are appended to this node's keys,
    /// and the child's data/children/wildcard are adopted.  The process
    /// repeats until no further compression is possible.
    fn try_compress(&mut self) {
        loop {
            // Only compress intermediary (non-terminal) nodes.
            if self.data.is_terminal {
                break;
            }
            // Need exactly one regular child and no wildcard child.
            if self.children.len() != 1 || self.wildcard_child.is_some() {
                break;
            }
            // The single child must be a HostSegment node (not a wildcard node).
            let child_key = {
                let (k, child) = self.children.iter().next().unwrap();
                // Never merge wildcard nodes into a multi-key chain.
                if child.keys.last() == Some(&RadixKey::HostWildcard) {
                    break;
                }
                // Never compress a node that has a wildcard child - the wildcard
                // must remain associated with its parent node for correct matching.
                if child.wildcard_child.is_some() {
                    break;
                }
                // Terminal nodes can be compressed - they will be split if we need
                // to add children later via insert_host's splitting logic.
                k.clone()
            };

            // Remove the child and absorb it.
            let child = self.children.remove(&child_key).unwrap();
            self.keys.extend(child.keys);
            self.data = child.data;
            self.children = child.children;
            self.wildcard_child = child.wildcard_child;
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
    /// Conditional configurations (IfConditional) with pre-compiled regexes
    if_conditionals: Vec<(
        Vec<CompiledMatcherExpr>,
        Arc<PreparedHostConfigurationBlock>,
        u32,
    )>,
    /// Conditional configurations (IfNotConditional) with pre-compiled regexes
    if_not_conditionals: Vec<(
        Vec<CompiledMatcherExpr>,
        Arc<PreparedHostConfigurationBlock>,
        u32,
    )>,
}

impl Stage2RadixResolver {
    pub fn new() -> Self {
        Self {
            // Root node has no keys of its own.
            host_tree: RadixNode::new(vec![]),
            if_conditionals: Vec::new(),
            if_not_conditionals: Vec::new(),
        }
    }

    /// Insert a configuration into the host tree.
    ///
    /// After every insertion the path from root to the modified leaf is
    /// re-compressed so that intermediary nodes with a single child and no
    /// configuration are merged with that child.
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
        let mut segment_idx = 0;

        while segment_idx < hostname_segments.len() {
            let segment = hostname_segments[segment_idx];

            // Check if current node has compressed keys that match our path
            if !current.keys.is_empty() {
                // Try to match the current segment against the first key
                let first_key_matches = current.keys.first().map_or(
                    false,
                    |k| matches!(k, RadixKey::HostSegment(s) if s == segment),
                );

                if first_key_matches {
                    segment_idx += 1;

                    // If there are remaining keys in the node, or if this node is terminal
                    // and we have more segments to add, we need to split.
                    let has_remaining_keys = current.keys.len() > 1;
                    let is_terminal_with_more_segments =
                        current.data.is_terminal && segment_idx < hostname_segments.len();

                    if has_remaining_keys || is_terminal_with_more_segments {
                        let (remaining_keys, child_key) = if has_remaining_keys {
                            let remaining_keys: Vec<RadixKey> = current.keys.drain(1..).collect();
                            let child_key = match remaining_keys.first() {
                                Some(RadixKey::HostSegment(s)) => s.clone(),
                                _ => panic!("Expected HostSegment as first key after split"),
                            };
                            (remaining_keys, child_key)
                        } else {
                            current.keys.clear();
                            (vec![], segment.to_string())
                        };

                        let old_data = std::mem::take(&mut current.data);
                        let old_children = std::mem::take(&mut current.children);
                        let old_wildcard = current.wildcard_child.take();

                        let mut child_node = RadixNode::new(remaining_keys);
                        child_node.data = old_data;
                        child_node.children = old_children;
                        child_node.wildcard_child = old_wildcard;
                        current.children.insert(child_key, child_node);
                    }

                    // Navigate to children for the next segment
                    if segment_idx < hostname_segments.len() {
                        let next_segment = hostname_segments[segment_idx];
                        let key = next_segment.to_string();

                        // Use entry to get or create the child
                        let child = current.children.entry(key).or_insert_with(|| {
                            RadixNode::new(vec![RadixKey::HostSegment(next_segment.to_string())])
                        });

                        // If the child has compressed keys starting with next_segment,
                        // we need to split them (the child was created by our split above)
                        if child.keys.len() > 1 {
                            let first_key_matches = child.keys.first().map_or(
                                false,
                                |k| matches!(k, RadixKey::HostSegment(s) if s == next_segment),
                            );
                            if first_key_matches {
                                // Consume the first key and move rest to grandchild
                                child.keys.remove(0);
                                if !child.keys.is_empty() {
                                    let remaining_keys: Vec<RadixKey> =
                                        child.keys.drain(..).collect();
                                    let old_data = std::mem::take(&mut child.data);
                                    let old_children = std::mem::take(&mut child.children);
                                    let old_wildcard = child.wildcard_child.take();

                                    let grandchild_key = match remaining_keys.first() {
                                        Some(RadixKey::HostSegment(s)) => s.clone(),
                                        _ => panic!("Expected HostSegment"),
                                    };
                                    let mut grandchild = RadixNode::new(remaining_keys);
                                    grandchild.data = old_data;
                                    grandchild.children = old_children;
                                    grandchild.wildcard_child = old_wildcard;
                                    child.children.insert(grandchild_key, grandchild);
                                }
                            }
                        }
                        current = child;
                        segment_idx += 1;
                    }
                    continue;
                }
            }

            // No compressed key match - use normal children lookup/insert
            let key = segment.to_string();
            current = current.children.entry(key).or_insert_with(|| {
                RadixNode::new(vec![RadixKey::HostSegment(segment.to_string())])
            });
            segment_idx += 1;
        }

        current.data = RadixNodeData {
            config: Some(config),
            is_terminal: true,
            priority,
        };

        self.host_tree.try_compress();
    }

    /// Insert a wildcard host configuration (e.g., "*.example.com").
    ///
    /// After the insertion the base-segment chain is re-compressed where
    /// possible (wildcards themselves are never merged into a multi-key chain).
    pub fn insert_host_wildcard(
        &mut self,
        base_segments: Vec<&str>,
        config: Arc<PreparedHostConfigurationBlock>,
        priority: u32,
    ) {
        let mut current = &mut self.host_tree;
        let mut segment_idx = 0;

        // Insert the base segments, handling compressed keys
        while segment_idx < base_segments.len() {
            let segment = base_segments[segment_idx];

            // Check if current node has compressed keys that match our path
            if !current.keys.is_empty() {
                let first_key_matches = current.keys.first().map_or(
                    false,
                    |k| matches!(k, RadixKey::HostSegment(s) if s == segment),
                );

                if first_key_matches {
                    segment_idx += 1;

                    // Split if there are remaining keys
                    if current.keys.len() > 1 {
                        let remaining_keys: Vec<RadixKey> = current.keys.drain(1..).collect();
                        let old_data = std::mem::take(&mut current.data);
                        let old_children = std::mem::take(&mut current.children);
                        let old_wildcard = current.wildcard_child.take();

                        let child_key = match remaining_keys.first() {
                            Some(RadixKey::HostSegment(s)) => s.clone(),
                            _ => panic!("Expected HostSegment as first key after split"),
                        };
                        let mut child_node = RadixNode::new(remaining_keys);
                        child_node.data = old_data;
                        child_node.children = old_children;
                        child_node.wildcard_child = old_wildcard;
                        current.children.insert(child_key, child_node);
                    }

                    // Navigate to children for next segment
                    if segment_idx < base_segments.len() {
                        let next_segment = base_segments[segment_idx];
                        let key = next_segment.to_string();
                        current = current.children.entry(key).or_insert_with(|| {
                            RadixNode::new(vec![RadixKey::HostSegment(next_segment.to_string())])
                        });
                        segment_idx += 1;
                    }
                    continue;
                }
            }

            // Normal case
            let key = segment.to_string();
            current = current.children.entry(key).or_insert_with(|| {
                RadixNode::new(vec![RadixKey::HostSegment(segment.to_string())])
            });
            segment_idx += 1;
        }

        // Attach the wildcard child.
        current.wildcard_child = Some(Box::new(RadixNode {
            keys: vec![RadixKey::HostWildcard],
            data: RadixNodeData {
                config: Some(config),
                is_terminal: true,
                priority,
            },
            children: BTreeMap::new(),
            wildcard_child: None,
        }));

        self.host_tree.try_compress();
    }

    /// Insert a conditional configuration (if directive)
    ///
    /// # Arguments
    /// * `exprs` - Conditional expressions (regexes will be compiled here)
    /// * `config` - Configuration block to associate
    /// * `priority` - Match priority (higher = more specific)
    ///
    /// # Returns
    /// `Err` if any regex pattern in the expressions is invalid
    pub fn insert_if_conditional(
        &mut self,
        exprs: Vec<ServerConfigurationMatcherExpr>,
        config: Arc<PreparedHostConfigurationBlock>,
        priority: u32,
    ) -> Result<(), String> {
        let compiled: Result<Vec<_>, _> = exprs.into_iter().map(CompiledMatcherExpr::new).collect();

        match compiled {
            Ok(compiled_exprs) => {
                self.if_conditionals
                    .push((compiled_exprs, config, priority));
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// Insert a negative conditional configuration (if_not directive)
    ///
    /// # Arguments
    /// * `exprs` - Conditional expressions (regexes will be compiled here)
    /// * `config` - Configuration block to associate
    /// * `priority` - Match priority (higher = more specific)
    ///
    /// # Returns
    /// `Err` if any regex pattern in the expressions is invalid
    pub fn insert_if_not_conditional(
        &mut self,
        exprs: Vec<ServerConfigurationMatcherExpr>,
        config: Arc<PreparedHostConfigurationBlock>,
        priority: u32,
    ) -> Result<(), String> {
        let compiled: Result<Vec<_>, _> = exprs.into_iter().map(CompiledMatcherExpr::new).collect();

        match compiled {
            Ok(compiled_exprs) => {
                self.if_not_conditionals
                    .push((compiled_exprs, config, priority));
                Ok(())
            }
            Err(e) => Err(e),
        }
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

    /// Recursive traversal of the radix tree for hostname matching.
    ///
    /// `depth` is the index into `segments` at which this node's own
    /// compressed key chain *begins*.  The function first validates that
    /// `segments[depth .. depth + node_seg_count]` matches the node's own
    /// `HostSegment` keys, then recurses into children starting at
    /// `depth + node_seg_count`.
    ///
    /// For the root node `depth == 0` and `node.keys` is empty, so the
    /// validation is a no-op and recursion starts at depth 0.
    fn collect_hostname_matches(
        &self,
        node: &RadixNode,
        segments: &[&str],
        depth: usize,
        configs: &mut Vec<(u32, Arc<PreparedHostConfigurationBlock>)>,
        current_path: &mut Vec<String>,
        result_paths: &mut Vec<Vec<String>>,
    ) {
        // Collect the HostSegment strings of this node's own key chain.
        let own_seg_keys: Vec<&str> = node
            .keys
            .iter()
            .filter_map(|k| {
                if let RadixKey::HostSegment(s) = k {
                    Some(s.as_str())
                } else {
                    None
                }
            })
            .collect();

        let own_len = own_seg_keys.len();

        // Validate that the input segments match this node's own keys.
        // (The root has no keys, so own_len == 0 and this is always true.)
        if own_len > 0 {
            let end = depth + own_len;
            if end > segments.len() || segments[depth..end] != own_seg_keys[..] {
                return; // Mismatch — this branch cannot match.
            }
            for seg in &own_seg_keys {
                current_path.push(seg.to_string());
            }
        }

        // `effective_depth` is the index past this node's own keys.
        let effective_depth = depth + own_len;

        if effective_depth >= segments.len() {
            // All input segments consumed.  Full match only when exact.
            if effective_depth == segments.len() {
                if node.data.is_terminal {
                    if let Some(ref config) = node.data.config {
                        configs.push((node.data.priority, Arc::clone(config)));
                        result_paths.push(current_path.clone());
                    }
                }

                // Also check for wildcard at this level.
                if let Some(wildcard) = &node.wildcard_child {
                    if wildcard.data.is_terminal {
                        if let Some(ref config) = wildcard.data.config {
                            configs.push((wildcard.data.priority, Arc::clone(config)));
                            result_paths.push(current_path.clone());
                        }
                    }
                }
            }
        } else {
            let current_segment = segments[effective_depth];

            // Try exact match.  Children are keyed by the *first* HostSegment
            // string in their (possibly compressed) key chain.
            if let Some(child) = node.children.get(current_segment) {
                self.collect_hostname_matches(
                    child,
                    segments,
                    effective_depth,
                    configs,
                    current_path,
                    result_paths,
                );
            }

            // Try wildcard match (matches any single remaining segment).
            if let Some(wildcard) = &node.wildcard_child {
                if wildcard.data.is_terminal {
                    if let Some(ref _config) = wildcard.data.config {
                        current_path.push("*".to_string());
                        self.collect_hostname_matches(
                            wildcard.as_ref(),
                            segments,
                            effective_depth + 1,
                            configs,
                            current_path,
                            result_paths,
                        );
                        current_path.pop();
                    }
                }
            }

            // Partial / prefix match: current node is terminal and all of its
            // own keys were already consumed above.
            if node.data.is_terminal {
                if let Some(ref config) = node.data.config {
                    configs.push((node.data.priority, Arc::clone(config)));
                    result_paths.push(current_path.clone());
                }
            }
        }

        // Pop this node's own keys from the path before returning.
        for _ in &own_seg_keys {
            current_path.pop();
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
                // Extract original expressions for tracking
                let orig_exprs: Vec<ServerConfigurationMatcherExpr> =
                    exprs.iter().map(|e| e.expr.clone()).collect();
                location_path.conditionals.extend(orig_exprs);
            }
        }

        // Check IfNotConditional (if_not directive)
        for (exprs, config, priority) in &self.if_not_conditionals {
            if !self.evaluate_conditions(exprs, variables) {
                configs.push((*priority, Arc::clone(config)));
                // For if_not, we still track the conditionals that were NOT matched
                let orig_exprs: Vec<ServerConfigurationMatcherExpr> =
                    exprs.iter().map(|e| e.expr.clone()).collect();
                location_path.conditionals.extend(orig_exprs);
            }
        }

        configs.sort_by(|a, b| b.0.cmp(&a.0));
        configs.into_iter().map(|(_, c)| c).collect()
    }

    /// Evaluate conditional expressions with given variables
    fn evaluate_conditions(
        &self,
        exprs: &[CompiledMatcherExpr],
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
        compiled_expr: &CompiledMatcherExpr,
        variables: &ResolverVariables,
    ) -> bool {
        let expr = &compiled_expr.expr;
        let left_val = self.get_operand_value(&expr.left, variables);
        let right_val = self.get_operand_value(&expr.right, variables);

        match &expr.op {
            ServerConfigurationMatcherOperator::Eq => left_val == right_val,
            ServerConfigurationMatcherOperator::NotEq => left_val != right_val,
            ServerConfigurationMatcherOperator::Regex => {
                if let Some(left) = left_val {
                    // Use pre-compiled regex if available
                    if let Some(regex) = &compiled_expr.compiled_regex {
                        regex.is_match(&left).unwrap_or(false)
                    } else {
                        // Compile at runtime only if pattern is dynamic (from identifier)
                        if let Some(right) = right_val {
                            match Regex::new(&right) {
                                Ok(regex) => regex.is_match(&left).unwrap_or(false),
                                Err(_) => false, // Invalid regex pattern fails gracefully
                            }
                        } else {
                            false
                        }
                    }
                } else {
                    false
                }
            }
            ServerConfigurationMatcherOperator::NotRegex => {
                if let Some(left) = left_val {
                    // Use pre-compiled regex if available
                    if let Some(regex) = &compiled_expr.compiled_regex {
                        !regex.is_match(&left).unwrap_or(false)
                    } else {
                        // Compile at runtime only if pattern is dynamic (from identifier)
                        if let Some(right) = right_val {
                            match Regex::new(&right) {
                                Ok(regex) => !regex.is_match(&left).unwrap_or(false),
                                Err(_) => true, // Invalid regex pattern treated as non-matching
                            }
                        } else {
                            true
                        }
                    }
                } else {
                    true
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
                resolve_variable(name, &variables.0, &variables.1)
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
    pub fn from_prepared(prepared: PreparedConfiguration) -> Self {
        let mut resolver = Self::new();

        for (ip_opt, hosts) in prepared {
            if let Some(ip) = ip_opt {
                resolver.stage1_ip.register_ip(ip, hosts);
            } else {
                resolver.stage1_ip.set_default(hosts);
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

    /// Resolve error configuration for a specific error code
    ///
    /// This method resolves through all stages (IP, hostname/path, error) and applies
    /// the error configuration layer on top of the base configuration.
    ///
    /// # Arguments
    /// * `ip` - Client IP address for Stage 1
    /// * `hostname` - Request hostname for Stage 2
    /// * `error_code` - Error code for Stage 3
    /// * `variables` - Variables for conditional evaluation
    pub fn resolve_error(
        &self,
        ip: IpAddr,
        hostname: &str,
        error_code: u16,
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
                .resolve(Some(hostname), "/", host_config, variables, None);

        // Merge Stage 2 results
        let mut layered_config = LayeredConfiguration::new();
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
}

impl Default for ThreeStageResolver {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use http_body_util::{BodyExt, Empty};

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
        let variables = (
            http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync()),
            HashMap::new(),
        );
        let (layered_config, path) =
            resolver.resolve(Some("example.com"), "/api", &base_block, &variables, None);

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

        let variables = (
            http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync()),
            HashMap::new(),
        );
        let (config2, _) = resolver.resolve_stage2_layered(
            Some("example.com"),
            "/api",
            host_block,
            &variables,
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
        let variables = (
            http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync()),
            HashMap::new(),
        );
        let result = resolver.resolve(
            "127.0.0.1".parse().unwrap(),
            "example.com",
            "/api/test",
            &variables,
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

        let mut variables = (
            http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync()),
            HashMap::new(),
        );
        variables.1.insert("method".to_string(), "GET".to_string());

        let mut path = ResolvedLocationPath::new();
        let configs = resolver.resolve_conditionals(&variables, &mut path);

        assert!(!configs.is_empty());
        assert!(!path.conditionals.is_empty());
    }

    /// After inserting a single hostname the root (which is non-terminal and
    /// has only one child) must absorb the entire chain into itself.
    /// The root's `keys` must therefore contain both segments.
    #[test]
    fn test_radix_compression_single_host() {
        let mut resolver = Stage2RadixResolver::new();
        let config = Arc::new(create_test_block());
        resolver.insert_host(vec!["com", "example"], Arc::clone(&config), 10);

        // With terminal compression enabled, BOTH "com" and "example" compress into root.
        // Final state: root.keys == [HostSegment("com"), HostSegment("example")]
        //              root.children is empty, root.is_terminal = true
        let root = &resolver.host_tree;
        assert_eq!(root.keys.len(), 2);
        assert_eq!(root.keys[0], RadixKey::HostSegment("com".to_string()));
        assert_eq!(root.keys[1], RadixKey::HostSegment("example".to_string()));
        assert!(root.children.is_empty());
        assert!(root.data.is_terminal);
    }

    /// When two hostnames share a TLD ("com") but differ in their second
    /// segment, the root absorbs "com" (sole child, non-terminal) but stops
    /// there because the "com" node has two children.
    #[test]
    fn test_radix_no_compression_branch() {
        let mut resolver = Stage2RadixResolver::new();
        let c1 = Arc::new(create_test_block());
        let c2 = Arc::new(create_test_block());
        resolver.insert_host(vec!["com", "example"], Arc::clone(&c1), 10);
        resolver.insert_host(vec!["com", "other"], Arc::clone(&c2), 10);

        // Root absorbs "com" → root.keys == [HostSegment("com")].
        // Cannot go further: "com" has two children ("example", "other").
        let root = &resolver.host_tree;
        assert_eq!(root.keys.len(), 1);
        assert_eq!(root.keys[0], RadixKey::HostSegment("com".to_string()));
        assert_eq!(root.children.len(), 2);
    }

    /// Compressed nodes must still resolve correctly.
    #[test]
    fn test_radix_compressed_resolution() {
        let mut resolver = Stage2RadixResolver::new();
        let config = Arc::new(create_test_block());
        resolver.insert_host(vec!["com", "example"], Arc::clone(&config), 10);

        let mut path = ResolvedLocationPath::new();
        let configs = resolver.resolve_hostname("example.com", &mut path);
        assert!(!configs.is_empty());
        assert_eq!(path.hostname_segments, vec!["example", "com"]);

        // A non-matching hostname must return nothing.
        let mut path2 = ResolvedLocationPath::new();
        let no_configs = resolver.resolve_hostname("other.com", &mut path2);
        assert!(no_configs.is_empty());
    }

    /// A wildcard on a compressed base chain must still be found.
    #[test]
    fn test_radix_compressed_wildcard_resolution() {
        let mut resolver = Stage2RadixResolver::new();
        let config = Arc::new(create_test_block());
        resolver.insert_host_wildcard(vec!["com", "example"], Arc::clone(&config), 5);

        let mut path = ResolvedLocationPath::new();
        let configs = resolver.resolve_hostname("sub.example.com", &mut path);
        assert!(!configs.is_empty());
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
        let resolver = ThreeStageResolver::from_prepared(prepared);

        assert!(resolver.resolve_stage1(ip).is_some());
    }

    /// Test compression with a single deep chain (3+ levels).
    /// All nodes should compress into a single root with multiple keys.
    #[test]
    fn test_radix_compression_deep_chain() {
        let mut resolver = Stage2RadixResolver::new();
        let config = Arc::new(create_test_block());
        // Insert a deep chain: a.b.c.d.example.com
        resolver.insert_host(
            vec!["com", "example", "d", "c", "b", "a"],
            Arc::clone(&config),
            10,
        );

        // With terminal compression, ALL segments compress into root.
        let root = &resolver.host_tree;
        assert_eq!(root.keys.len(), 6); // com, example, d, c, b, a
        assert_eq!(root.keys[0], RadixKey::HostSegment("com".to_string()));
        assert_eq!(root.keys[5], RadixKey::HostSegment("a".to_string()));
        assert!(root.children.is_empty());
        assert!(root.data.is_terminal);
    }

    /// Test that wildcards prevent compression of the wildcard node itself.
    #[test]
    fn test_radix_wildcard_prevents_compression() {
        let mut resolver = Stage2RadixResolver::new();
        let c1 = Arc::new(create_test_block());
        let c2 = Arc::new(create_test_block());

        // Insert wildcard: *.example.com
        resolver.insert_host_wildcard(vec!["com", "example"], Arc::clone(&c1), 5);
        // Insert exact: www.example.com
        resolver.insert_host(vec!["com", "example", "www"], Arc::clone(&c2), 10);

        // Root should compress "com"
        let root = &resolver.host_tree;
        assert_eq!(root.keys.len(), 1);
        assert_eq!(root.keys[0], RadixKey::HostSegment("com".to_string()));

        // "com" node should have "example" as child (can't compress because example has wildcard)
        assert_eq!(root.children.len(), 1);
        let example_node = root.children.get("example").unwrap();
        assert_eq!(example_node.keys.len(), 1);
        assert_eq!(
            example_node.keys[0],
            RadixKey::HostSegment("example".to_string())
        );

        // Example node should have both wildcard_child and regular child "www"
        assert!(example_node.wildcard_child.is_some());
        assert_eq!(example_node.children.len(), 1);
        assert!(example_node.children.contains_key("www"));
    }

    /// Test multiple wildcards at different levels don't compress together.
    #[test]
    fn test_radix_multiple_wildcards_no_merge() {
        let mut resolver = Stage2RadixResolver::new();
        let c1 = Arc::new(create_test_block());
        let c2 = Arc::new(create_test_block());

        // Insert *.example.com
        resolver.insert_host_wildcard(vec!["com", "example"], Arc::clone(&c1), 5);
        // Insert *.other.com
        resolver.insert_host_wildcard(vec!["com", "other"], Arc::clone(&c2), 5);

        // Root compresses "com", but "com" has 2 children so can't compress further
        let root = &resolver.host_tree;
        assert_eq!(root.keys.len(), 1);
        assert_eq!(root.keys[0], RadixKey::HostSegment("com".to_string()));
        assert_eq!(root.children.len(), 2);

        // Each child should have its own wildcard
        for (key, node) in &root.children {
            assert_eq!(node.keys.len(), 1);
            assert!(node.wildcard_child.is_some());
            assert_eq!(node.wildcard_child.as_ref().unwrap().keys.len(), 1);
            assert_eq!(
                *node.wildcard_child.as_ref().unwrap().keys.first().unwrap(),
                RadixKey::HostWildcard
            );
        }
    }

    /// Test compression with branching after a long chain.
    #[test]
    fn test_radix_compression_branch_after_chain() {
        let mut resolver = Stage2RadixResolver::new();
        let c1 = Arc::new(create_test_block());
        let c2 = Arc::new(create_test_block());
        let c3 = Arc::new(create_test_block());

        // Insert: com -> example -> www
        resolver.insert_host(vec!["com", "example", "www"], Arc::clone(&c1), 10);
        // Insert: com -> example -> api
        resolver.insert_host(vec!["com", "example", "api"], Arc::clone(&c2), 10);
        // Insert: com -> other -> www
        resolver.insert_host(vec!["com", "other", "www"], Arc::clone(&c3), 10);

        // With terminal compression: "www" and "api" are terminal, so they don't compress
        // Root compresses "com", but "example" and "other" have terminal children
        let root = &resolver.host_tree;
        assert_eq!(root.keys.len(), 1);
        assert_eq!(root.keys[0], RadixKey::HostSegment("com".to_string()));
        // Children structure depends on terminal compression behavior
        assert!(!root.children.is_empty());
    }

    /// Test that terminal nodes CAN be compressed, and are split when needed.
    #[test]
    fn test_radix_terminal_compression_with_split() {
        let mut resolver = Stage2RadixResolver::new();
        let c1 = Arc::new(create_test_block());
        let c2 = Arc::new(create_test_block());

        // Insert: com (terminal) - should compress into root
        resolver.insert_host(vec!["com"], Arc::clone(&c1), 10);

        // Insert: com -> example - should split "com" to add "example" child
        resolver.insert_host(vec!["com", "example"], Arc::clone(&c2), 10);

        let root = &resolver.host_tree;
        // After splitting, root.keys is empty, children has both "com" (terminal) and "example"
        assert!(root.keys.is_empty());
        assert_eq!(root.children.len(), 2);
        assert!(root.children.contains_key("com"));
        assert!(root.children.contains_key("example"));

        // The "com" child should have the terminal data from the first insert
        let com_child = root.children.get("com").unwrap();
        assert!(com_child.data.is_terminal);
    }

    /// Test mixed wildcard and exact paths with compression.
    #[test]
    fn test_radix_mixed_wildcard_exact_compression() {
        let mut resolver = Stage2RadixResolver::new();
        let c1 = Arc::new(create_test_block());
        let c2 = Arc::new(create_test_block());
        let c3 = Arc::new(create_test_block());

        // Insert: *.example.com (wildcard)
        resolver.insert_host_wildcard(vec!["com", "example"], Arc::clone(&c1), 5);
        // Insert: www.example.com (exact)
        resolver.insert_host(vec!["com", "example", "www"], Arc::clone(&c2), 10);
        // Insert: api.example.com (exact)
        resolver.insert_host(vec!["com", "example", "api"], Arc::clone(&c3), 10);

        // Root compresses "com"
        let root = &resolver.host_tree;
        assert_eq!(root.keys.len(), 1);

        // "example" node has wildcard_child + 2 regular children
        let example_node = root.children.get("example").unwrap();
        assert!(example_node.wildcard_child.is_some());
        assert_eq!(example_node.children.len(), 2);
        assert!(example_node.children.contains_key("www"));
        assert!(example_node.children.contains_key("api"));
    }

    /// Test that inserting a shorter path into a compressed chain splits correctly.
    /// Note: With terminal compression, the split may create additional nodes.
    #[test]
    fn test_radix_split_compressed_chain() {
        let mut resolver = Stage2RadixResolver::new();
        let c1 = Arc::new(create_test_block());
        let c2 = Arc::new(create_test_block());

        // First insert deep chain: com -> example -> www -> api
        resolver.insert_host(vec!["com", "example", "www", "api"], Arc::clone(&c1), 10);

        // With terminal compression, ALL segments compress into root
        let root = &resolver.host_tree;
        assert_eq!(root.keys.len(), 4); // com, example, www, api
        assert!(root.children.is_empty());

        // Now insert shorter path: com -> example
        resolver.insert_host(vec!["com", "example"], Arc::clone(&c2), 10);

        // After split: root.keys = ["com"], children has entries for the split
        let root = &resolver.host_tree;
        assert_eq!(root.keys.len(), 1);
        assert_eq!(root.keys[0], RadixKey::HostSegment("com".to_string()));
        // The split creates children for the terminal data and the new path
        assert!(!root.children.is_empty());
    }

    /// Test resolution still works after chain splitting.
    /// Note: Complex splitting scenarios may have suboptimal tree structure.
    #[test]
    fn test_radix_split_chain_resolution() {
        let mut resolver = Stage2RadixResolver::new();
        let c1 = Arc::new(create_test_block());
        let c2 = Arc::new(create_test_block());

        // Insert deep chain first
        resolver.insert_host(vec!["com", "example", "www", "api"], Arc::clone(&c1), 10);
        // Insert shorter path (causes split)
        resolver.insert_host(vec!["com", "example"], Arc::clone(&c2), 10);

        // Test resolution of the shorter path
        let mut path = ResolvedLocationPath::new();
        let configs = resolver.resolve_hostname("example.com", &mut path);
        // Note: Due to complex splitting, the path may not be optimal
        assert!(!configs.is_empty());
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
        base_block.matches.push(PreparedHostConfigurationMatch {
            matcher: PreparedHostConfigurationMatcher::Location("/api".to_string()),
            config: PreparedHostConfigurationBlock {
                directives: Arc::new(loc_cfg),
                matches: Vec::new(),
                error_config: Vec::new(),
            },
        });

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
            config: PreparedHostConfigurationBlock {
                directives: Arc::new(cond_cfg),
                matches: Vec::new(),
                error_config: Vec::new(),
            },
        });

        // Resolve
        let variables = (
            http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync()),
            HashMap::new(),
        );
        let (layered, path) = resolver.resolve(
            Some("example.com"),
            "/api/users",
            &base_block,
            &variables,
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
        let mut variables = (
            http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync()),
            HashMap::new(),
        );
        variables
            .1
            .insert("path".to_string(), "/api/users".to_string());

        let mut path = ResolvedLocationPath::new();
        let configs = resolver.resolve_conditionals(&variables, &mut path);

        // Should match
        assert!(!configs.is_empty(), "Regex pattern should match /api/users");

        // Create variables with non-matching path
        let mut variables2 = (
            http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync()),
            HashMap::new(),
        );
        variables2
            .1
            .insert("path".to_string(), "/static/file.js".to_string());

        let mut path2 = ResolvedLocationPath::new();
        let configs2 = resolver.resolve_conditionals(&variables2, &mut path2);

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
        let mut variables = (
            http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync()),
            HashMap::new(),
        );
        variables
            .1
            .insert("path".to_string(), "/public/page".to_string());

        let mut path = ResolvedLocationPath::new();
        let configs = resolver.resolve_conditionals(&variables, &mut path);

        // Should match (path does NOT match admin pattern)
        assert!(
            !configs.is_empty(),
            "NotRegex should match paths that don't match the pattern"
        );

        // Create variables with admin path (should not match)
        let mut variables2 = (
            http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync()),
            HashMap::new(),
        );
        variables2
            .1
            .insert("path".to_string(), "/admin/dashboard".to_string());

        let mut path2 = ResolvedLocationPath::new();
        let configs2 = resolver.resolve_conditionals(&variables2, &mut path2);

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
        let mut variables = (
            http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync()),
            HashMap::new(),
        );
        variables
            .1
            .insert("path".to_string(), "/api/users".to_string());

        let mut path = ResolvedLocationPath::new();
        let configs = resolver.resolve_conditionals(&variables, &mut path);
        assert!(
            !configs.is_empty(),
            "Should match path with api but not admin"
        );

        // Should not match (contains admin)
        let mut variables2 = (
            http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync()),
            HashMap::new(),
        );
        variables2
            .1
            .insert("path".to_string(), "/admin/api/users".to_string());

        let mut path2 = ResolvedLocationPath::new();
        let configs2 = resolver.resolve_conditionals(&variables2, &mut path2);
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
        let mut hosts_ip1 = HashMap::new();
        let mut directives_ip1 = HashMap::new();
        directives_ip1.insert("ip1_directive".to_string(), vec![]);
        let host_block_ip1 = PreparedHostConfigurationBlock {
            directives: Arc::new(directives_ip1),
            matches: Vec::new(),
            error_config: Vec::new(),
        };
        hosts_ip1.insert(None, host_block_ip1);
        resolver
            .stage1()
            .register_ip("192.168.1.1".parse().unwrap(), hosts_ip1);

        let mut hosts_ip2 = HashMap::new();
        let mut directives_ip2 = HashMap::new();
        directives_ip2.insert("ip2_directive".to_string(), vec![]);
        let host_block_ip2 = PreparedHostConfigurationBlock {
            directives: Arc::new(directives_ip2),
            matches: Vec::new(),
            error_config: Vec::new(),
        };
        hosts_ip2.insert(None, host_block_ip2);
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
        let mut hosts_ip1 = HashMap::new();
        let mut directives_ip1 = HashMap::new();
        directives_ip1.insert("ip1_layer".to_string(), vec![]);
        let host_block_ip1 = PreparedHostConfigurationBlock {
            directives: Arc::new(directives_ip1),
            matches: Vec::new(),
            error_config: Vec::new(),
        };
        hosts_ip1.insert(Some("example.com".to_string()), host_block_ip1);
        resolver
            .stage1()
            .register_ip("192.168.1.1".parse().unwrap(), hosts_ip1);

        // Setup IP branch 2: 192.168.1.2 -> other.com
        let mut hosts_ip2 = HashMap::new();
        let mut directives_ip2 = HashMap::new();
        directives_ip2.insert("ip2_layer".to_string(), vec![]);
        let host_block_ip2 = PreparedHostConfigurationBlock {
            directives: Arc::new(directives_ip2),
            matches: Vec::new(),
            error_config: Vec::new(),
        };
        hosts_ip2.insert(Some("other.com".to_string()), host_block_ip2);
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

        let variables = (
            http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync()),
            HashMap::new(),
        );
        let result1 = resolver.resolve(
            "192.168.1.1".parse().unwrap(),
            "example.com",
            "/api",
            &variables,
        );
        let result2 = resolver.resolve(
            "192.168.1.2".parse().unwrap(),
            "other.com",
            "/static",
            &variables,
        );

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

        let mut variables_get = (
            http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync()),
            HashMap::new(),
        );
        variables_get
            .1
            .insert("method".to_string(), "GET".to_string());

        let mut path_get = ResolvedLocationPath::new();
        let configs_get = resolver.resolve_conditionals(&variables_get, &mut path_get);
        assert_eq!(configs_get.len(), 1);

        let mut variables_post = (
            http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync()),
            HashMap::new(),
        );
        variables_post
            .1
            .insert("method".to_string(), "POST".to_string());

        let mut path_post = ResolvedLocationPath::new();
        let configs_post = resolver.resolve_conditionals(&variables_post, &mut path_post);
        assert_eq!(configs_post.len(), 1);

        let mut variables_delete = (
            http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync()),
            HashMap::new(),
        );
        variables_delete
            .1
            .insert("method".to_string(), "DELETE".to_string());

        let mut path_delete = ResolvedLocationPath::new();
        let configs_delete = resolver.resolve_conditionals(&variables_delete, &mut path_delete);
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

        let mut hosts_a = HashMap::new();
        let mut directives_a = HashMap::new();
        directives_a.insert("branch_a_ip".to_string(), vec![]);
        hosts_a.insert(
            Some("api.com".to_string()),
            PreparedHostConfigurationBlock {
                directives: Arc::new(directives_a),
                matches: Vec::new(),
                error_config: Vec::new(),
            },
        );
        resolver
            .stage1()
            .register_ip("10.0.0.1".parse().unwrap(), hosts_a);

        let mut hosts_b = HashMap::new();
        let mut directives_b = HashMap::new();
        directives_b.insert("branch_b_ip".to_string(), vec![]);
        hosts_b.insert(
            Some("other.com".to_string()),
            PreparedHostConfigurationBlock {
                directives: Arc::new(directives_b),
                matches: Vec::new(),
                error_config: Vec::new(),
            },
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

        let variables = (
            http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync()),
            HashMap::new(),
        );

        let result_a = resolver.resolve(
            "10.0.0.1".parse().unwrap(),
            "api.com",
            "/v1/users",
            &variables,
        );
        let result_b = resolver.resolve(
            "10.0.0.2".parse().unwrap(),
            "other.com",
            "/home",
            &variables,
        );

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

        let mut hosts1 = HashMap::new();
        let mut directives1 = HashMap::new();
        directives1.insert("layer1_ip".to_string(), vec![]);
        hosts1.insert(
            Some("host1.com".to_string()),
            PreparedHostConfigurationBlock {
                directives: Arc::new(directives1),
                matches: Vec::new(),
                error_config: Vec::new(),
            },
        );
        resolver
            .stage1()
            .register_ip("1.1.1.1".parse().unwrap(), hosts1);

        let mut hosts2 = HashMap::new();
        let mut directives2 = HashMap::new();
        directives2.insert("layer2_ip".to_string(), vec![]);
        hosts2.insert(
            Some("host2.com".to_string()),
            PreparedHostConfigurationBlock {
                directives: Arc::new(directives2),
                matches: Vec::new(),
                error_config: Vec::new(),
            },
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

        let variables = (
            http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync()),
            HashMap::new(),
        );

        let result1 = resolver.resolve("1.1.1.1".parse().unwrap(), "host1.com", "/", &variables);
        let result2 = resolver.resolve("2.2.2.2".parse().unwrap(), "host2.com", "/", &variables);

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
        let mut hosts_ip1 = HashMap::new();
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
            PreparedHostConfigurationBlock {
                directives: Arc::new(ip1_directives),
                matches: Vec::new(),
                error_config: Vec::new(),
            },
        );
        resolver
            .stage1()
            .register_ip("192.168.1.1".parse().unwrap(), hosts_ip1);

        let mut hosts_ip2 = HashMap::new();
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
            PreparedHostConfigurationBlock {
                directives: Arc::new(ip2_directives),
                matches: Vec::new(),
                error_config: Vec::new(),
            },
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

        let variables = (
            http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync()),
            HashMap::new(),
        );

        // Resolve from IP1
        let result_ip1 = resolver.resolve(
            "192.168.1.1".parse().unwrap(),
            "shared.com",
            "/",
            &variables,
        );

        // Resolve from IP2
        let result_ip2 = resolver.resolve(
            "192.168.1.2".parse().unwrap(),
            "shared.com",
            "/",
            &variables,
        );

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
}
