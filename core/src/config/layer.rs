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
        for layer in self.layers.iter().rev() {
            if let Some(entry) = layer
                .directives
                .get(directive)
                .and_then(|entries| entries.last())
            {
                return Some(entry);
            }
            if !inherit {
                break;
            }
        }
        None
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
        for layer in self.layers.iter().rev() {
            if let Some(value) = layer
                .directives
                .get(directive)
                .and_then(|entries| entries.last())
                .and_then(|entry| entry.args.first())
            {
                return Some(value);
            }
            if !inherit {
                break;
            }
        }
        None
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
        for layer in self.layers.iter().rev() {
            if let Some(entry) = layer
                .directives
                .get(directive)
                .and_then(|entries| entries.last())
            {
                if let Some(crate::config::ServerConfigurationValue::Boolean(value, _)) =
                    entry.args.first()
                {
                    return *value;
                }
                return true;
            }
            if !inherit {
                break;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::LayeredConfiguration;
    use crate::config::ServerConfigurationBlockBuilder;

    #[test]
    fn get_value_prefers_last_entry_in_highest_priority_layer() {
        let low = ServerConfigurationBlockBuilder::new()
            .directive_str("root", vec!["/srv/low"])
            .build();
        let high = ServerConfigurationBlockBuilder::new()
            .directive_str("root", vec!["/srv/high-initial"])
            .directive_str("root", vec!["/srv/high-final"])
            .build();

        let mut layered = LayeredConfiguration::new();
        layered.add_layer(std::sync::Arc::new(low));
        layered.add_layer(std::sync::Arc::new(high));

        assert_eq!(
            layered
                .get_value("root", true)
                .and_then(|value| value.as_str()),
            Some("/srv/high-final")
        );
    }

    #[test]
    fn get_value_without_inheritance_only_checks_highest_priority_layer() {
        let low = ServerConfigurationBlockBuilder::new()
            .directive_str("root", vec!["/srv/low"])
            .build();
        let high = ServerConfigurationBlockBuilder::new().build();

        let mut layered = LayeredConfiguration::new();
        layered.add_layer(std::sync::Arc::new(low));
        layered.add_layer(std::sync::Arc::new(high));

        assert!(layered.get_value("root", false).is_none());
        assert_eq!(
            layered
                .get_value("root", true)
                .and_then(|value| value.as_str()),
            Some("/srv/low")
        );
    }
}
