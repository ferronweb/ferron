use std::collections::BTreeMap;
use std::sync::Arc;

use ferron_core::config::layer::LayeredConfiguration;

/// A single OpenTelemetry span link configuration.
///
/// Span links connect causally related spans that do not have a direct
/// parent-child relationship. For example, a control plane event that
/// triggers multiple data plane requests would link to each resulting
/// request span.
#[derive(Clone, Debug)]
pub struct SpanLinkConfig {
    /// The trace ID of the linked span (32 hex chars).
    pub trace_id: String,
    /// The span ID of the linked span (16 hex chars).
    pub span_id: String,
    /// Whether the linked span was sampled (defaults to false).
    pub sampled: bool,
    /// Attributes describing the relationship.
    pub attributes: Arc<BTreeMap<String, String>>,
}

/// Configuration for cross-plane traceability metadata.
///
/// The `control_plane` block allows the control plane (e.g. a Kubernetes
/// ingress controller) to embed arbitrary key-value pairs and static OpenTelemetry
/// span links in the server configuration. Metadata values are automatically
/// included as attributes on all observability signals (traces, logs, metrics,
/// access logs) under the `ferron.control_plane.*` namespace. Span links
/// establish causal relationships between control plane events and data plane
/// traces.
///
/// The directive can appear at global, host, or location levels. When present
/// at multiple levels, the most specific level wins (location > host > global).
#[derive(Clone, Debug, Default)]
pub struct ControlPlaneConfig {
    /// Arbitrary key-value metadata defined by the control plane.
    pub metadata: Arc<BTreeMap<String, String>>,
    /// Static OpenTelemetry span links defined by the control plane.
    pub span_links: Arc<Vec<SpanLinkConfig>>,
}

impl ControlPlaneConfig {
    /// Extract `control_plane` metadata from a parsed configuration block.
    ///
    /// Looks for a `control_plane` directive with a `metadata` sub-block and
    /// collects all `key "value"` pairs inside it. Also extracts any `span_links`
    /// sub-blocks. Returns `None` when neither the `control_plane` block nor
    /// `metadata` is present.
    pub fn from_block(block: &ferron_core::config::ServerConfigurationBlock) -> Option<Self> {
        let cp_entry = block.directives.get("control_plane")?.first()?;
        let cp_children = cp_entry.children.as_ref()?;
        Some(Self::extract(cp_children))
    }

    /// Extract `control_plane` metadata from a layered configuration.
    ///
    /// Uses the most specific (highest priority) `control_plane` directive
    /// across all layers. This implements the precedence: location > host > global.
    pub fn from_layered(config: &LayeredConfiguration) -> Option<Self> {
        let entries = config.get_entries("control_plane", true);
        let entry = entries.first()?;
        let children = entry.children.as_ref()?;
        Some(Self::extract(children))
    }

    /// Extract both metadata and span links from a `control_plane` block's children.
    fn extract(cp_children: &ferron_core::config::ServerConfigurationBlock) -> Self {
        let metadata = Self::extract_metadata(cp_children).unwrap_or_default();
        let span_links = Self::extract_span_links(cp_children);
        Self {
            metadata,
            span_links,
        }
    }

    /// Extract metadata key-value pairs from a `control_plane` block's children.
    ///
    /// Expects a block containing a `metadata` sub-block with `key "value"` directives.
    fn extract_metadata(
        cp_children: &ferron_core::config::ServerConfigurationBlock,
    ) -> Option<Arc<BTreeMap<String, String>>> {
        let metadata_entries = cp_children.directives.get("metadata")?;
        let metadata_entry = metadata_entries.first()?;
        let metadata_children = metadata_entry.children.as_ref()?;

        let mut metadata = BTreeMap::new();
        for (key, entries) in metadata_children.directives.iter() {
            if let Some(entry) = entries.first() {
                if let Some(value) = entry
                    .args
                    .first()
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                {
                    metadata.insert(key.clone(), value);
                }
            }
        }

        if metadata.is_empty() {
            return None;
        }

        Some(Arc::new(metadata))
    }

    /// Extract span links from `span_links { ... }` sub-blocks.
    ///
    /// Each `span_links` block defines one link with `trace_id`, `span_id`,
    /// optional `sampled`, and optional `attributes`.
    fn extract_span_links(
        cp_children: &ferron_core::config::ServerConfigurationBlock,
    ) -> Arc<Vec<SpanLinkConfig>> {
        let Some(entries) = cp_children.directives.get("span_links") else {
            return Arc::new(Vec::new());
        };

        let mut links = Vec::new();
        for entry in entries {
            let Some(children) = entry.children.as_ref() else {
                continue;
            };

            let trace_id = children
                .get_value("trace_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let span_id = children
                .get_value("span_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let sampled = children
                .get_value("sampled")
                .and_then(|v| v.as_boolean())
                .unwrap_or(false);

            let mut attributes = BTreeMap::new();
            if let Some(attrs_entries) = children.directives.get("attributes") {
                if let Some(attrs_entry) = attrs_entries.first() {
                    if let Some(attrs_children) = attrs_entry.children.as_ref() {
                        for (key, key_entries) in attrs_children.directives.iter() {
                            if let Some(key_entry) = key_entries.first() {
                                if let Some(value) = key_entry
                                    .args
                                    .first()
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string())
                                {
                                    attributes.insert(key.clone(), value);
                                }
                            }
                        }
                    }
                }
            }

            links.push(SpanLinkConfig {
                trace_id,
                span_id,
                sampled,
                attributes: Arc::new(attributes),
            });
        }

        Arc::new(links)
    }

    /// Returns `true` if there is at least one metadata entry.
    pub fn has_metadata(&self) -> bool {
        !self.metadata.is_empty()
    }

    /// Returns `true` if there is at least one span link.
    pub fn has_span_links(&self) -> bool {
        !self.span_links.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::config::{ServerConfigurationBlock, ServerConfigurationDirectiveEntry};
    use std::collections::HashMap;

    fn make_value(s: &str) -> ferron_core::config::ServerConfigurationValue {
        ferron_core::config::ServerConfigurationValue::String(s.to_string(), None)
    }

    fn make_entry_with_children(
        children: ServerConfigurationBlock,
    ) -> ServerConfigurationDirectiveEntry {
        ServerConfigurationDirectiveEntry {
            args: vec![],
            children: Some(children),
            span: None,
        }
    }

    fn make_block(
        directives: HashMap<String, Vec<ServerConfigurationDirectiveEntry>>,
    ) -> ServerConfigurationBlock {
        ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: HashMap::new(),
            span: None,
        }
    }

    fn build_metadata_block(pairs: &[(&str, &str)]) -> ServerConfigurationBlock {
        let mut metadata_directives = HashMap::new();
        for (key, value) in pairs {
            metadata_directives.insert(
                key.to_string(),
                vec![ServerConfigurationDirectiveEntry {
                    args: vec![make_value(value)],
                    children: None,
                    span: None,
                }],
            );
        }
        let metadata_block = make_block(metadata_directives);

        let mut cp_directives = HashMap::new();
        cp_directives.insert(
            "metadata".to_string(),
            vec![make_entry_with_children(metadata_block)],
        );
        let cp_block = make_block(cp_directives);

        let mut root_directives = HashMap::new();
        root_directives.insert(
            "control_plane".to_string(),
            vec![make_entry_with_children(cp_block)],
        );
        make_block(root_directives)
    }

    fn build_cp_block_with_span_links(links: &[(&str, &str)]) -> ServerConfigurationBlock {
        let mut cp_directives = HashMap::new();
        for (trace_id, span_id) in links {
            let mut link_directives = HashMap::new();
            link_directives.insert(
                "trace_id".to_string(),
                vec![ServerConfigurationDirectiveEntry {
                    args: vec![make_value(trace_id)],
                    children: None,
                    span: None,
                }],
            );
            link_directives.insert(
                "span_id".to_string(),
                vec![ServerConfigurationDirectiveEntry {
                    args: vec![make_value(span_id)],
                    children: None,
                    span: None,
                }],
            );
            cp_directives
                .entry("span_links".to_string())
                .or_insert_with(Vec::new)
                .push(make_entry_with_children(make_block(link_directives)));
        }
        let cp_block = make_block(cp_directives);

        let mut root_directives = HashMap::new();
        root_directives.insert(
            "control_plane".to_string(),
            vec![make_entry_with_children(cp_block)],
        );
        make_block(root_directives)
    }

    fn build_cp_block_with_attrs(attrs: &[(&str, &str)]) -> ServerConfigurationBlock {
        let mut cp_directives = HashMap::new();

        // Build attributes block for the first span_links entry
        let mut attrs_directives = HashMap::new();
        for (key, value) in attrs {
            attrs_directives.insert(
                key.to_string(),
                vec![ServerConfigurationDirectiveEntry {
                    args: vec![make_value(value)],
                    children: None,
                    span: None,
                }],
            );
        }
        let attrs_block = make_block(attrs_directives);

        let mut link_directives = HashMap::new();
        link_directives.insert(
            "trace_id".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")],
                children: None,
                span: None,
            }],
        );
        link_directives.insert(
            "span_id".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value("bbbbbbbbbbbbbbbb")],
                children: None,
                span: None,
            }],
        );
        link_directives.insert(
            "sampled".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ferron_core::config::ServerConfigurationValue::Boolean(
                    true, None,
                )],
                children: None,
                span: None,
            }],
        );
        link_directives.insert(
            "attributes".to_string(),
            vec![make_entry_with_children(attrs_block)],
        );
        cp_directives.insert(
            "span_links".to_string(),
            vec![make_entry_with_children(make_block(link_directives))],
        );

        let cp_block = make_block(cp_directives);

        let mut root_directives = HashMap::new();
        root_directives.insert(
            "control_plane".to_string(),
            vec![make_entry_with_children(cp_block)],
        );
        make_block(root_directives)
    }

    #[test]
    fn from_block_extracts_metadata() {
        let block = build_metadata_block(&[("org_id", "12345"), ("team", "platform")]);
        let config = ControlPlaneConfig::from_block(&block).unwrap();
        assert!(config.has_metadata());
        assert_eq!(config.metadata.get("org_id").unwrap(), "12345");
        assert_eq!(config.metadata.get("team").unwrap(), "platform");
    }

    #[test]
    fn from_block_returns_none_when_missing() {
        let block = make_block(HashMap::new());
        assert!(ControlPlaneConfig::from_block(&block).is_none());
    }

    #[test]
    fn from_block_returns_none_when_empty_metadata_and_no_span_links() {
        let mut cp_directives = HashMap::new();
        cp_directives.insert(
            "metadata".to_string(),
            vec![make_entry_with_children(make_block(HashMap::new()))],
        );
        let cp_block = make_block(cp_directives);

        let mut root_directives = HashMap::new();
        root_directives.insert(
            "control_plane".to_string(),
            vec![make_entry_with_children(cp_block)],
        );
        let block = make_block(root_directives);
        let config = ControlPlaneConfig::from_block(&block).unwrap();
        assert!(!config.has_metadata());
        assert!(!config.has_span_links());
    }

    #[test]
    fn from_block_no_metadata_sub_block_only_span_links() {
        let mut cp_directives = HashMap::new();
        cp_directives.insert(
            "something_else".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value("value")],
                children: None,
                span: None,
            }],
        );
        let cp_block = make_block(cp_directives);

        let mut root_directives = HashMap::new();
        root_directives.insert(
            "control_plane".to_string(),
            vec![make_entry_with_children(cp_block)],
        );
        let block = make_block(root_directives);
        let config = ControlPlaneConfig::from_block(&block).unwrap();
        assert!(!config.has_metadata());
        assert!(!config.has_span_links());
    }

    #[test]
    fn has_metadata_true_when_nonempty() {
        let config = ControlPlaneConfig {
            metadata: Arc::new({
                let mut m = BTreeMap::new();
                m.insert("key".to_string(), "value".to_string());
                m
            }),
            ..Default::default()
        };
        assert!(config.has_metadata());
    }

    #[test]
    fn has_metadata_false_when_empty() {
        let config = ControlPlaneConfig::default();
        assert!(!config.has_metadata());
    }

    #[test]
    fn from_block_extracts_span_links() {
        let block = build_cp_block_with_span_links(&[
            ("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbb"),
            ("cccccccccccccccccccccccccccccccc", "dddddddddddddddd"),
        ]);
        let config = ControlPlaneConfig::from_block(&block).unwrap();
        assert!(config.has_span_links());
        assert_eq!(config.span_links.len(), 2);
        assert_eq!(
            config.span_links[0].trace_id,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(config.span_links[0].span_id, "bbbbbbbbbbbbbbbb");
        assert_eq!(
            config.span_links[1].trace_id,
            "cccccccccccccccccccccccccccccccc"
        );
        assert_eq!(config.span_links[1].span_id, "dddddddddddddddd");
    }

    #[test]
    fn from_block_span_links_defaults() {
        let block = build_cp_block_with_span_links(&[(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "bbbbbbbbbbbbbbbb",
        )]);
        let config = ControlPlaneConfig::from_block(&block).unwrap();
        assert!(!config.span_links[0].sampled);
        assert!(config.span_links[0].attributes.is_empty());
    }

    #[test]
    fn from_block_span_links_with_attrs() {
        let block = build_cp_block_with_attrs(&[
            ("relationship", "triggers"),
            ("source", "ingress-controller"),
        ]);
        let config = ControlPlaneConfig::from_block(&block).unwrap();
        assert!(config.has_span_links());
        let link = &config.span_links[0];
        assert_eq!(link.trace_id, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        assert_eq!(link.span_id, "bbbbbbbbbbbbbbbb");
        assert!(link.sampled);
        assert_eq!(link.attributes.get("relationship").unwrap(), "triggers");
        assert_eq!(link.attributes.get("source").unwrap(), "ingress-controller");
    }

    #[test]
    fn from_block_no_span_links_when_absent() {
        let block = build_metadata_block(&[("org_id", "12345")]);
        let config = ControlPlaneConfig::from_block(&block).unwrap();
        assert!(!config.has_span_links());
        assert!(config.span_links.is_empty());
    }

    #[test]
    fn has_span_links_false_when_empty() {
        let config = ControlPlaneConfig::default();
        assert!(!config.has_span_links());
    }

    #[test]
    fn from_block_metadata_and_span_links_combined() {
        let mut cp_directives = HashMap::new();

        // Metadata
        let mut metadata_directives = HashMap::new();
        metadata_directives.insert(
            "org_id".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value("12345")],
                children: None,
                span: None,
            }],
        );
        cp_directives.insert(
            "metadata".to_string(),
            vec![make_entry_with_children(make_block(metadata_directives))],
        );

        // Span link
        let mut link_directives = HashMap::new();
        link_directives.insert(
            "trace_id".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")],
                children: None,
                span: None,
            }],
        );
        link_directives.insert(
            "span_id".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value("bbbbbbbbbbbbbbbb")],
                children: None,
                span: None,
            }],
        );
        cp_directives.insert(
            "span_links".to_string(),
            vec![make_entry_with_children(make_block(link_directives))],
        );

        let cp_block = make_block(cp_directives);
        let mut root_directives = HashMap::new();
        root_directives.insert(
            "control_plane".to_string(),
            vec![make_entry_with_children(cp_block)],
        );
        let block = make_block(root_directives);

        let config = ControlPlaneConfig::from_block(&block).unwrap();
        assert!(config.has_metadata());
        assert!(config.has_span_links());
        assert_eq!(config.metadata.get("org_id").unwrap(), "12345");
        assert_eq!(config.span_links.len(), 1);
    }
}
