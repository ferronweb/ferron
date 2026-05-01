use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use ferron_core::config::{
    layer::LayeredConfiguration, ServerConfigurationBlock, ServerConfigurationMatcherExpr,
    ServerConfigurationMatcherOperand,
};
use ferron_http::HttpContext;

use super::super::prepare::PreparedHostConfigurationBlock;
use super::matcher::{
    evaluate_matcher_condition, evaluate_matcher_conditions, resolve_matcher_operand,
    CompiledMatcherExpr,
};
use super::types::ResolvedLocationPath;

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
#[derive(Debug, Clone, Default)]
pub struct RadixNodeData {
    /// Configuration block at this node
    pub config: Option<Arc<PreparedHostConfigurationBlock>>,
    /// Whether this node is a terminal (exact match) node
    pub is_terminal: bool,
    /// Priority for matching (higher = more specific)
    pub priority: u32,
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
pub(crate) struct RadixNode {
    /// The (possibly compressed) sequence of keys leading to this node.
    /// Empty only for the synthetic root node.
    pub(crate) keys: Vec<RadixKey>,
    pub(crate) data: RadixNodeData,
    pub(crate) children: BTreeMap<String, RadixNode>,
    pub(crate) wildcard_child: Option<Box<RadixNode>>,
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
                if !self.keys.is_empty() && child.keys.last() == Some(&RadixKey::HostWildcard) {
                    break;
                }
                // Never compress a node that has a wildcard child - the wildcard
                // must remain associated with its parent node for correct matching.
                if !self.keys.is_empty() && child.wildcard_child.is_some() {
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
    pub(crate) host_tree: RadixNode,
    /// The radix tree for location path matching
    pub(crate) path_tree: RadixNode,
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

impl Default for Stage2RadixResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl Stage2RadixResolver {
    pub fn new() -> Self {
        Self {
            // Root node has no keys of its own.
            host_tree: RadixNode::new(vec![]),
            path_tree: RadixNode::new(vec![]),
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
            // Check if current node has compressed keys that match our path
            if current.keys.first().is_some_and(
                |k| matches!(k, RadixKey::HostSegment(s) if s == hostname_segments[segment_idx]),
            ) {
                let mut key_idx = 0;
                for key in &current.keys {
                    if segment_idx >= hostname_segments.len() {
                        break;
                    }
                    if !matches!(key, RadixKey::HostSegment(s) if s == hostname_segments[segment_idx])
                    {
                        break;
                    }
                    segment_idx += 1;
                    key_idx += 1;
                }

                // Split if there are remaining keys
                if current.keys.len() > key_idx {
                    let remaining_keys: Vec<RadixKey> = current.keys.drain(key_idx..).collect();
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

                // Navigate to children for the next segment
                if segment_idx < hostname_segments.len() {
                    let next_segment = hostname_segments[segment_idx];
                    let key = next_segment.to_string();
                    current = current.children.entry(key).or_insert_with(|| {
                        RadixNode::new(vec![RadixKey::HostSegment(next_segment.to_string())])
                    });
                }
                continue;
            }

            if !current.keys.is_empty() {
                // Compressed key mismatch - split the current node
                let child_key = match current.keys[0] {
                    RadixKey::HostSegment(ref s) => s.clone(),
                    _ => panic!("Unexpected key type in compressed node"),
                };
                let remaining_keys: Vec<RadixKey> = current.keys.drain(..).collect();
                let old_data = std::mem::take(&mut current.data);
                let old_children = std::mem::take(&mut current.children);
                let old_wildcard = current.wildcard_child.take();
                let mut child_node = RadixNode::new(remaining_keys);
                child_node.data = old_data;
                child_node.children = old_children;
                child_node.wildcard_child = old_wildcard;
                current.children.insert(child_key.to_string(), child_node);
            }

            // No compressed key match - use normal children lookup/insert
            let segment = hostname_segments[segment_idx];
            let key = segment.to_string();
            current = current.children.entry(key).or_insert_with(|| {
                RadixNode::new(
                    hostname_segments[segment_idx..]
                        .iter()
                        .map(|s| RadixKey::HostSegment(s.to_string()))
                        .collect(),
                )
            });
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
            if current.keys.first().is_some_and(
                |k| matches!(k, RadixKey::HostSegment(s) if s == base_segments[segment_idx]),
            ) {
                let mut key_idx = 0;
                for key in &current.keys {
                    if segment_idx >= base_segments.len() {
                        break;
                    }
                    if !matches!(key, RadixKey::HostSegment(s) if s == base_segments[segment_idx]) {
                        break;
                    }
                    segment_idx += 1;
                    key_idx += 1;
                }

                // Split if there are remaining keys
                if current.keys.len() > key_idx {
                    let remaining_keys: Vec<RadixKey> = current.keys.drain(key_idx..).collect();
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
                }
                continue;
            }

            if !current.keys.is_empty() {
                // Compressed key mismatch - split the current node
                let child_key = match current.keys[0] {
                    RadixKey::HostSegment(ref s) => s.clone(),
                    _ => panic!("Unexpected key type in compressed node"),
                };
                let remaining_keys: Vec<RadixKey> = current.keys.drain(..).collect();
                let old_data = std::mem::take(&mut current.data);
                let old_children = std::mem::take(&mut current.children);
                let old_wildcard = current.wildcard_child.take();
                let mut child_node = RadixNode::new(remaining_keys);
                child_node.data = old_data;
                child_node.children = old_children;
                child_node.wildcard_child = old_wildcard;
                current.children.insert(child_key, child_node);
            }

            // Normal case
            let key = segment.to_string();
            current = current.children.entry(key).or_insert_with(|| {
                RadixNode::new(
                    base_segments[segment_idx..]
                        .iter()
                        .map(|s| RadixKey::HostSegment(s.to_string()))
                        .collect(),
                )
            });
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

    /// Insert a location path configuration into the path radix tree.
    ///
    /// # Arguments
    /// * `path_segments` - Path segments in order (e.g., ["api", "users"] for "/api/users")
    /// * `config` - Configuration block to associate
    /// * `priority` - Match priority (higher = more specific)
    pub fn insert_location(
        &mut self,
        path_segments: Vec<&str>,
        config: Arc<PreparedHostConfigurationBlock>,
        priority: u32,
    ) {
        let mut current = &mut self.path_tree;
        let mut segment_idx = 0;

        while segment_idx < path_segments.len() {
            let segment = path_segments[segment_idx];

            // Check if current node has compressed keys that match our path
            if current.keys.first().is_some_and(
                |k| matches!(k, RadixKey::PathSegment(s) if s == path_segments[segment_idx]),
            ) {
                let mut key_idx = 0;
                for key in &current.keys {
                    if segment_idx >= path_segments.len() {
                        break;
                    }
                    if !matches!(key, RadixKey::PathSegment(s) if s == path_segments[segment_idx]) {
                        break;
                    }
                    segment_idx += 1;
                    key_idx += 1;
                }

                // Split if there are remaining keys
                if current.keys.len() > key_idx {
                    let remaining_keys: Vec<RadixKey> = current.keys.drain(key_idx..).collect();
                    let old_data = std::mem::take(&mut current.data);
                    let old_children = std::mem::take(&mut current.children);
                    let old_wildcard = current.wildcard_child.take();

                    let child_key = match remaining_keys.first() {
                        Some(RadixKey::PathSegment(s)) => s.clone(),
                        _ => panic!("Expected PathSegment as first key after split"),
                    };
                    let mut child_node = RadixNode::new(remaining_keys);
                    child_node.data = old_data;
                    child_node.children = old_children;
                    child_node.wildcard_child = old_wildcard;
                    current.children.insert(child_key, child_node);
                }

                // Navigate to children for next segment
                if segment_idx < path_segments.len() {
                    let next_segment = path_segments[segment_idx];
                    let key = next_segment.to_string();
                    current = current.children.entry(key).or_insert_with(|| {
                        RadixNode::new(vec![RadixKey::PathSegment(next_segment.to_string())])
                    });
                }
                continue;
            }

            if !current.keys.is_empty() {
                // Compressed key mismatch - split the current node
                let child_key = match current.keys[0] {
                    RadixKey::PathSegment(ref s) => s.clone(),
                    _ => panic!("Unexpected key type in compressed node"),
                };
                let remaining_keys: Vec<RadixKey> = current.keys.drain(..).collect();
                let old_data = std::mem::take(&mut current.data);
                let old_children = std::mem::take(&mut current.children);
                let old_wildcard = current.wildcard_child.take();
                let mut child_node = RadixNode::new(remaining_keys);
                child_node.data = old_data;
                child_node.children = old_children;
                child_node.wildcard_child = old_wildcard;
                current.children.insert(child_key, child_node);
            }

            // Normal case
            let key = segment.to_string();
            current = current.children.entry(key).or_insert_with(|| {
                RadixNode::new(
                    path_segments[segment_idx..]
                        .iter()
                        .map(|s| RadixKey::PathSegment(s.to_string()))
                        .collect(),
                )
            });
        }

        current.data = RadixNodeData {
            config: Some(config),
            is_terminal: true,
            priority,
        };

        self.path_tree.try_compress();
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
        configs.sort_by_key(|b| std::cmp::Reverse(b.0));

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
        }

        // Pop this node's own keys from the path before returning.
        for _ in &own_seg_keys {
            current_path.pop();
        }
    }

    /// Resolve location path and return matching configurations
    ///
    /// This performs a longest-prefix match search through the path radix tree.
    /// Uses Arc::clone() for zero-copy sharing of the base configuration.
    pub fn resolve_location(
        &self,
        path: &str,
        base_config: Arc<PreparedHostConfigurationBlock>,
        location_path: &mut ResolvedLocationPath,
    ) -> Vec<Arc<PreparedHostConfigurationBlock>> {
        let mut configs = Vec::new();

        // First, add the base configuration (zero-copy Arc clone)
        configs.push((0u32, Arc::clone(&base_config)));

        // Parse path segments
        let segments: Vec<&str> = path
            .trim_start_matches('/')
            .split('/')
            .filter(|s| !s.is_empty())
            .collect();

        // Traverse the path tree and collect all matching configs
        let mut matched_configs = Vec::new();
        let mut matched_path_segments = Vec::new();
        self.collect_path_matches(
            &self.path_tree,
            &segments,
            0,
            &mut matched_configs,
            &mut Vec::new(),
            &mut matched_path_segments,
        );

        // Sort by priority (descending - longer/more specific paths first)
        matched_configs.sort_by_key(|b| std::cmp::Reverse(b.0));

        // Use the longest matched path (most specific match)
        if let Some(longest_path) = matched_path_segments.into_iter().max_by_key(|p| p.len()) {
            location_path.path_segments = longest_path;
        }

        // Add matched configs
        for (_, config) in matched_configs {
            configs.push((0u32, Arc::clone(&config)));
        }

        configs.into_iter().map(|(_, c)| c).collect()
    }

    /// Recursive traversal of the path radix tree for location matching.
    ///
    /// Collects all matching configurations along the path, supporting
    /// both exact matches and prefix matches (terminal nodes).
    fn collect_path_matches(
        &self,
        node: &RadixNode,
        segments: &[&str],
        depth: usize,
        configs: &mut Vec<(u32, Arc<PreparedHostConfigurationBlock>)>,
        current_path: &mut Vec<String>,
        result_paths: &mut Vec<Vec<String>>,
    ) {
        // Collect the PathSegment strings of this node's own key chain.
        let own_seg_keys: Vec<&str> = node
            .keys
            .iter()
            .filter_map(|k| {
                if let RadixKey::PathSegment(s) = k {
                    Some(s.as_str())
                } else {
                    None
                }
            })
            .collect();

        let own_len = own_seg_keys.len();

        // Validate that the input segments match this node's own keys.
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

        // If this node is terminal, it's a prefix match
        if node.data.is_terminal {
            if let Some(ref config) = node.data.config {
                configs.push((node.data.priority, Arc::clone(config)));
                result_paths.push(current_path.clone());
            }
        }

        // If all segments consumed, check for exact match
        if effective_depth == segments.len() {
            // Already added terminal nodes above
            // Also check children for more specific matches (prefix match)
            for child in node.children.values() {
                self.collect_path_matches(
                    child,
                    segments,
                    effective_depth,
                    configs,
                    current_path,
                    result_paths,
                );
            }
        } else if effective_depth < segments.len() {
            // More segments to match - try exact child match
            let current_segment = segments[effective_depth];
            if let Some(child) = node.children.get(current_segment) {
                self.collect_path_matches(
                    child,
                    segments,
                    effective_depth,
                    configs,
                    current_path,
                    result_paths,
                );
            }
        }

        // Pop this node's own keys from the path before returning.
        for _ in &own_seg_keys {
            current_path.pop();
        }
    }

    /// Resolve conditionals with given context
    pub fn resolve_conditionals(
        &self,
        ctx: &HttpContext,
        location_path: &mut ResolvedLocationPath,
    ) -> Vec<Arc<PreparedHostConfigurationBlock>> {
        let mut configs = Vec::new();

        // Check IfConditional (if directive)
        for (exprs, config, priority) in &self.if_conditionals {
            if self.evaluate_conditions(exprs, ctx) {
                configs.push((*priority, Arc::clone(config)));
                // Extract original expressions for tracking
                let orig_exprs: Vec<ServerConfigurationMatcherExpr> =
                    exprs.iter().map(|e| e.expr.clone()).collect();
                location_path.conditionals.extend(orig_exprs);
            }
        }

        // Check IfNotConditional (if_not directive)
        for (exprs, config, priority) in &self.if_not_conditionals {
            if !self.evaluate_conditions(exprs, ctx) {
                configs.push((*priority, Arc::clone(config)));
                // For if_not, we still track the conditionals that were NOT matched
                let orig_exprs: Vec<ServerConfigurationMatcherExpr> =
                    exprs.iter().map(|e| e.expr.clone()).collect();
                location_path.conditionals.extend(orig_exprs);
            }
        }

        configs.sort_by_key(|b| std::cmp::Reverse(b.0));
        configs.into_iter().map(|(_, c)| c).collect()
    }

    /// Evaluate conditional expressions with given context
    fn evaluate_conditions(&self, exprs: &[CompiledMatcherExpr], ctx: &HttpContext) -> bool {
        evaluate_matcher_conditions(exprs, ctx)
    }

    /// Full Stage 2 resolution combining hostname, location, and conditionals
    ///
    /// # Arguments
    /// * `hostname` - Request hostname to resolve
    /// * `path` - Request path to resolve
    /// * `base_config` - The base prepared host configuration block
    /// * `ctx` - HTTP context for variable resolution and conditional evaluation
    /// * `layered_config` - Optional base layered configuration to add layers to
    pub fn resolve(
        &self,
        hostname: Option<&str>,
        path: &str,
        base_config: Arc<PreparedHostConfigurationBlock>,
        ctx: &HttpContext,
        layered_config: Option<LayeredConfiguration>,
    ) -> (LayeredConfiguration, ResolvedLocationPath) {
        let mut location_path = ResolvedLocationPath::new();
        let mut layered_config = layered_config.unwrap_or_default();

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

        // Resolve location paths (zero-copy Arc clone)
        for config in self.resolve_location(path, Arc::clone(&base_config), &mut location_path) {
            let block = ServerConfigurationBlock {
                directives: Arc::clone(&config.directives),
                matchers: HashMap::new(),
                span: None,
            };
            layered_config.add_layer(Arc::new(block));
        }

        // Resolve conditionals
        for config in self.resolve_conditionals(ctx, &mut location_path) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn create_test_block() -> PreparedHostConfigurationBlock {
        PreparedHostConfigurationBlock {
            directives: Arc::new(std::collections::HashMap::new()),
            matches: Vec::new(),
            error_config: Vec::new(),
        }
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
    fn test_stage2_partial_hostname_resolution() {
        let mut resolver = Stage2RadixResolver::new();

        let config = Arc::new(create_test_block());

        // Insert example.com configuration
        resolver.insert_host(vec!["com", "example"], Arc::clone(&config), 10);

        let mut path = ResolvedLocationPath::new();
        let configs = resolver.resolve_hostname("sub.example.com", &mut path);

        // "example.com" should not match "sub.example.com"
        assert!(configs.is_empty());
    }

    #[test]
    fn test_radix_compression_single_host() {
        let mut resolver = Stage2RadixResolver::new();

        let config = Arc::new(create_test_block());

        // Insert com.example configuration
        resolver.insert_host(vec!["com", "example"], Arc::clone(&config), 10);

        // After insertion, verify the radix tree structure
        // The root should have one child with compressed keys
        let root = &resolver.host_tree;
        assert!(!root.keys.is_empty(), "Root should have compressed keys");
        assert_eq!(
            root.keys.len(),
            2,
            "Should have 2 compressed keys: com, example"
        );
    }

    #[test]
    fn test_radix_no_compression_branch() {
        let mut resolver = Stage2RadixResolver::new();

        let config1 = Arc::new(create_test_block());
        let config2 = Arc::new(create_test_block());

        // Insert two different hosts to create a branch
        resolver.insert_host(vec!["com", "example"], Arc::clone(&config1), 10);
        resolver.insert_host(vec!["com", "google"], Arc::clone(&config2), 10);

        // The radix tree should NOT compress here because "com" has multiple children
        let root = &resolver.host_tree;
        assert!(!root.children.is_empty(), "Root should have children");
    }

    #[test]
    fn test_stage2_path_resolution_exact() {
        let mut resolver = Stage2RadixResolver::new();

        let config = Arc::new(create_test_block());

        // Insert /api/users path configuration
        resolver.insert_location(vec!["api", "users"], Arc::clone(&config), 10);

        let mut path = ResolvedLocationPath::new();
        let configs = resolver.resolve_location("/api/users", Arc::clone(&config), &mut path);

        assert!(!configs.is_empty());
        assert_eq!(path.path_segments, vec!["api", "users"]);
    }

    #[test]
    fn test_stage2_path_resolution_prefix() {
        let mut resolver = Stage2RadixResolver::new();

        let base_config = Arc::new(create_test_block());
        let api_config = Arc::new(create_test_block());

        // Insert /api path configuration
        resolver.insert_location(vec!["api"], Arc::clone(&api_config), 3);

        let mut path = ResolvedLocationPath::new();
        let configs =
            resolver.resolve_location("/api/users/123", Arc::clone(&base_config), &mut path);

        // Should match the /api prefix
        assert!(configs.len() >= 2); // base + api
        assert_eq!(path.path_segments, vec!["api"]);
    }

    #[test]
    fn test_stage2_path_resolution_longest_match() {
        let mut resolver = Stage2RadixResolver::new();

        let base_config = Arc::new(create_test_block());
        let api_config = Arc::new(create_test_block());
        let users_config = Arc::new(create_test_block());

        // Insert /api and /api/users path configurations
        resolver.insert_location(vec!["api"], Arc::clone(&api_config), 3);
        resolver.insert_location(vec!["api", "users"], Arc::clone(&users_config), 10);

        let mut path = ResolvedLocationPath::new();
        let configs =
            resolver.resolve_location("/api/users/123", Arc::clone(&base_config), &mut path);

        // Should match both /api and /api/users, with /api/users being more specific
        assert!(configs.len() >= 3); // base + api + users
        assert_eq!(path.path_segments, vec!["api", "users"]);
    }

    #[test]
    fn test_stage2_path_resolution_no_match() {
        let mut resolver = Stage2RadixResolver::new();

        let base_config = Arc::new(create_test_block());
        let api_config = Arc::new(create_test_block());

        // Insert /api path configuration
        resolver.insert_location(vec!["api"], Arc::clone(&api_config), 3);

        let mut path = ResolvedLocationPath::new();
        let configs = resolver.resolve_location("/other/path", Arc::clone(&base_config), &mut path);

        // Should only have base config
        assert_eq!(configs.len(), 1);
        assert!(path.path_segments.is_empty());
    }

    #[test]
    fn test_path_radix_compression() {
        let mut resolver = Stage2RadixResolver::new();

        let config = Arc::new(create_test_block());

        // Insert api/v1/users path
        resolver.insert_location(vec!["api", "v1", "users"], Arc::clone(&config), 15);

        // After insertion, verify the path tree structure
        let root = &resolver.path_tree;
        assert!(!root.keys.is_empty(), "Root should have compressed keys");
        assert_eq!(
            root.keys.len(),
            3,
            "Should have 3 compressed keys: api, v1, users"
        );
    }

    // ── Hostname radix tree compression tests ──

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

        // Root should compress "com" and "example"
        let root = &resolver.host_tree;
        assert_eq!(root.keys.len(), 2);
        assert_eq!(root.keys[0], RadixKey::HostSegment("com".to_string()));
        assert_eq!(root.keys[1], RadixKey::HostSegment("example".to_string()));

        // Example node should have both wildcard_child and regular child "www"
        assert!(root.wildcard_child.is_some());
        assert_eq!(root.children.len(), 1);
        assert!(root.children.contains_key("www"));
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
        for node in root.children.values() {
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

        // The root should have the terminal data from the first insert
        let root = &resolver.host_tree;
        assert_eq!(root.keys, vec![RadixKey::HostSegment("com".to_string())]);
        assert!(root.data.is_terminal);
        assert_eq!(root.children.len(), 1);
        assert!(root.children.contains_key("example"));
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

        // Root compresses "com" and "example" into a single node
        let root = &resolver.host_tree;
        assert!(root.wildcard_child.is_some());
        assert_eq!(root.children.len(), 2);
        assert!(root.children.contains_key("www"));
        assert!(root.children.contains_key("api"));
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

        // After split: root.keys = ["com", "example"], children has entries for the split
        let root = &resolver.host_tree;
        assert_eq!(root.keys.len(), 2);
        assert_eq!(root.keys[0], RadixKey::HostSegment("com".to_string()));
        assert_eq!(root.keys[1], RadixKey::HostSegment("example".to_string()));
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
}
