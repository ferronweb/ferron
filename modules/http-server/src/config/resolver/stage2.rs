use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use ferron_core::config::{
    layer::LayeredConfiguration, ServerConfigurationBlock, ServerConfigurationMatcherExpr,
    ServerConfigurationMatcherOperand,
};

use super::super::prepare::{PreparedHostConfigurationBlock, PreparedHostConfigurationMatcher};
use super::matcher::{
    evaluate_matcher_condition, evaluate_matcher_conditions, resolve_matcher_operand,
    CompiledMatcherExpr,
};
use super::types::{ResolvedLocationPath, ResolverVariables};

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
    pub(crate) host_tree: RadixNode,
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
                let first_key_matches = current
                    .keys
                    .first()
                    .is_some_and(|k| matches!(k, RadixKey::HostSegment(s) if s == segment));

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
                            let first_key_matches = child.keys.first().is_some_and(
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
                let first_key_matches = current
                    .keys
                    .first()
                    .is_some_and(|k| matches!(k, RadixKey::HostSegment(s) if s == segment));

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
    /// This performs a prefix-based search through the location matches.
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

        // Find matching location directives
        for location_match in &base_config.matches {
            if let PreparedHostConfigurationMatcher::Location(location_path_str) =
                &location_match.matcher
            {
                if self.location_matches(path, location_path_str) {
                    // Calculate priority based on specificity (longer path = more specific)
                    let priority = location_path_str.len() as u32;
                    configs.push((priority, Arc::clone(&location_match.config)));

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
        evaluate_matcher_conditions(exprs, variables)
    }

    /// Evaluate a single conditional expression with given variables
    fn evaluate_condition(
        &self,
        compiled_expr: &CompiledMatcherExpr,
        variables: &ResolverVariables,
    ) -> bool {
        evaluate_matcher_condition(compiled_expr, variables)
    }

    /// Get the string value of an operand
    fn get_operand_value(
        &self,
        operand: &ServerConfigurationMatcherOperand,
        variables: &ResolverVariables,
    ) -> Option<String> {
        resolve_matcher_operand(operand, variables)
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
        base_config: Arc<PreparedHostConfigurationBlock>,
        variables: &ResolverVariables,
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
}
