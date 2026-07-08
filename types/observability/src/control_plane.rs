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
