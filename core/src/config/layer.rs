//! Layered configuration support for hierarchical configuration inheritance.
//!
//! Layered configurations allow directives to be overridden at different levels
//! (global, protocol, host) with proper inheritance.

use std::sync::Arc;

/// A configuration composed of multiple layers for inheritance.
///
/// Layers are searched in reverse order (last added first) to implement
/// override semantics. This supports configuration hierarchies where:
/// - Global directives provide defaults
/// - Protocol-level directives override globals
/// - Host-level directives override protocol-level
#[derive(Clone, Default)]
pub struct LayeredConfiguration {
    /// Configuration layers, searched in reverse order
    pub layers: Vec<Arc<crate::config::ServerConfigurationBlock>>,
}

impl LayeredConfiguration {
    /// Create a new empty layered configuration.
    #[inline]
    pub fn new() -> Self {
        Self { layers: Vec::new() }
    }

    /// Add a configuration layer.
    ///
    /// Layers are searched in reverse order, so this layer will have higher priority
    /// than previously added layers.
    #[inline]
    pub fn add_layer(&mut self, layer: Arc<crate::config::ServerConfigurationBlock>) {
        self.layers.push(layer);
    }

    /// Get all entries for a directive across layers.
    ///
    /// # Arguments
    ///
    /// * `directive` - The directive name to search for
    /// * `inherit` - If true, search all layers in reverse order. If false, search only the first layer.
    ///
    /// # Returns
    ///
    /// A vector of all matching entries, with higher-priority (more recent) layers first.
    #[inline]
    pub fn get_entries<'a>(
        &'a self,
        directive: &str,
        inherit: bool,
    ) -> Vec<&'a crate::config::ServerConfigurationDirectiveEntry> {
        let mut entries = Vec::new();
        for layer in self.layers.iter().rev() {
            if let Some(directives) = layer.directives.get(directive) {
                entries.extend(directives);
            }
            if !inherit {
                break;
            }
        }
        entries
    }

    /// Get the first entry for a directive across layers.
    ///
    /// # Arguments
    ///
    /// * `directive` - The directive name to search for
    /// * `inherit` - If true, search all layers. If false, search only the first layer.
    ///
    /// # Returns
    ///
    /// The highest-priority (most recent) matching entry, or None if not found.
    #[inline]
    pub fn get_entry<'a>(
        &'a self,
        directive: &str,
        inherit: bool,
    ) -> Option<&'a crate::config::ServerConfigurationDirectiveEntry> {
        let mut entries = self.get_entries(directive, inherit);
        entries.pop()
    }

    /// Get the first value for a directive across layers.
    ///
    /// # Arguments
    ///
    /// * `directive` - The directive name to search for
    /// * `inherit` - If true, search all layers. If false, search only the first layer.
    ///
    /// # Returns
    ///
    /// The first argument value of the highest-priority matching entry.
    #[inline]
    pub fn get_value(
        &self,
        directive: &str,
        inherit: bool,
    ) -> Option<&crate::config::ServerConfigurationValue> {
        if let Some(entry) = self.get_entry(directive, inherit) {
            entry.args.first()
        } else {
            None
        }
    }

    /// Get a directive as a boolean flag across layers.
    ///
    /// # Arguments
    ///
    /// * `directive` - The directive name to search for
    /// * `inherit` - If true, search all layers. If false, search only the first layer.
    ///
    /// # Returns
    ///
    /// The boolean value if found, or true as default for flag-style directives.
    #[inline]
    pub fn get_flag(&self, directive: &str, inherit: bool) -> bool {
        if let Some(entry) = self.get_entry(directive, inherit) {
            if let Some(crate::config::ServerConfigurationValue::Boolean(value, _)) =
                entry.args.first()
            {
                return *value;
            } else {
                return true;
            }
        }
        false
    }
}
