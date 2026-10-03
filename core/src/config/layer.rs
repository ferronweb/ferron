//! Layered configuration support for hierarchical configuration inheritance.
//!
//! A [`LayeredConfiguration`] merges multiple
//! [`ServerConfigurationBlock`](crate::config::ServerConfigurationBlock)s with override semantics: the last added
//! layer has the highest priority. This supports the configuration
//! hierarchy where:
//!
//! - Global directives provide defaults.
//! - Protocol-level directives override globals.
//! - Host-level directives override protocol-level.
//!
//! # Example
//!
//! ```ignore
//! use std::sync::Arc;
//! use ferron_core::config::layer::LayeredConfiguration;
//!
//! let mut layered = LayeredConfiguration::new();
//! layered.add_layer(Arc::new(global_block));   // lowest priority
//! layered.add_layer(Arc::new(host_block));     // highest priority
//!
//! // "root" resolves from host_block if present, otherwise global_block
//! let root = layered.get_value("root", true)
//!     .and_then(|v| v.as_str());
//! ```

use std::sync::{Arc, LazyLock};

static EMPTY_LAYERS: LazyLock<Arc<Vec<Arc<crate::config::ServerConfigurationBlock>>>> =
    LazyLock::new(|| Arc::new(Vec::new()));

/// A configuration composed of multiple layers for inheritance.
///
/// Layers are stored in order and searched in reverse (last added first) to
/// implement override semantics. This supports configuration hierarchies
/// where more specific layers override less specific ones.
///
/// # Example
///
/// ```ignore
/// use std::sync::Arc;
/// use ferron_core::config::layer::LayeredConfiguration;
///
/// let mut layered = LayeredConfiguration::new();
/// layered.add_layer(Arc::new(global_block));
/// layered.add_layer(Arc::new(host_block));
///
/// let root = layered.get_value("root", true)
///     .and_then(|v| v.as_str());
/// ```
#[derive(Clone)]
pub struct LayeredConfiguration {
    /// Configuration layers, searched in reverse order
    pub layers: Arc<Vec<Arc<crate::config::ServerConfigurationBlock>>>,
    /// Layer index to start skipping no-inherit rules.
    skip_noinherit_from: usize,
    /// Number of leading layers that belong to global scope.
    ///
    /// Global layers stay inheritable even when `inherit` is `false`; that
    /// flag only cuts less-specific *host* layers (for example the wildcard
    /// `*` host underneath a named host).
    global_layer_count: usize,
}

impl Default for LayeredConfiguration {
    #[inline]
    fn default() -> Self {
        Self {
            layers: EMPTY_LAYERS.clone(),
            skip_noinherit_from: usize::MAX,
            global_layer_count: 0,
        }
    }
}

impl LayeredConfiguration {
    /// Create a new empty layered configuration.
    #[inline]
    pub fn new() -> Self {
        Self {
            layers: EMPTY_LAYERS.clone(),
            skip_noinherit_from: usize::MAX,
            global_layer_count: 0,
        }
    }

    /// Mark the current layer index to skip no-inherit rules.
    #[inline]
    pub fn mark_current_skip_noinherit(&mut self) {
        self.skip_noinherit_from = self.layers.len();
    }

    /// Mark the layers added so far as global-scope layers.
    ///
    /// The resolver calls this after adding the global configuration block
    /// and before adding host layers, so that `inherit = false` lookups still
    /// fall back to global defaults while skipping less-specific host layers.
    #[inline]
    pub fn mark_end_of_global_layers(&mut self) {
        self.global_layer_count = self.layers.len();
    }

    /// Add a configuration layer.
    ///
    /// New layers are appended to the end and have higher priority than
    /// previously added layers when `inherit` is `true`.
    #[inline]
    pub fn add_layer(&mut self, layer: Arc<crate::config::ServerConfigurationBlock>) {
        Arc::make_mut(&mut self.layers).push(layer);
    }

    /// Get all entries for a directive across layers.
    ///
    /// # Arguments
    ///
    /// * `directive` -- The directive name to search for.
    /// * `inherit` -- If `true`, search all layers in reverse order (highest
    ///   priority first). If `false`, search the host chain (the matched host
    ///   plus nested `location`/`if` layers) and then global-scope layers,
    ///   skipping less-specific host layers such as the wildcard `*` host.
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
        for (i, layer) in self.layers.iter().enumerate().rev() {
            if let Some(directives) = layer.directives.get(directive) {
                entries.extend(directives);
            }
            if !inherit && i < self.skip_noinherit_from {
                entries.extend(self.global_fallback_entries(directive));
                break;
            }
        }
        entries
    }

    /// Get entries for a directive from the host chain only.
    ///
    /// This covers the matched host plus nested `location`/`if` layers,
    /// excluding both less-specific host layers and global-scope layers.
    /// It mirrors the layers a `get_*` call with `inherit = false` checks,
    /// minus the global-scope fallback.
    #[inline]
    pub fn get_host_chain_entries<'a>(
        &'a self,
        directive: &str,
    ) -> Vec<&'a crate::config::ServerConfigurationDirectiveEntry> {
        let mut entries = Vec::new();
        if self.layers.is_empty() {
            return entries;
        }
        let chain_start = self
            .skip_noinherit_from
            .min(self.layers.len())
            .saturating_sub(1);
        for layer in self.layers[chain_start..].iter().rev() {
            if let Some(directives) = layer.directives.get(directive) {
                entries.extend(directives);
            }
        }
        entries
    }

    /// Entries for a directive from global-scope layers only.
    #[inline]
    fn global_fallback_entries<'a>(
        &'a self,
        directive: &str,
    ) -> Vec<&'a crate::config::ServerConfigurationDirectiveEntry> {
        let mut entries = Vec::new();
        for layer in self.layers.iter().take(self.global_layer_count).rev() {
            if let Some(directives) = layer.directives.get(directive) {
                entries.extend(directives);
            }
        }
        entries
    }

    /// Get the first entry for a directive across layers.
    ///
    /// Returns the highest-priority matching entry, or `None` if not found.
    /// When `inherit` is `false`, only the host chain (matched host plus
    /// nested layers) and global-scope layers are checked; less-specific
    /// host layers are skipped.
    #[inline]
    pub fn get_entry<'a>(
        &'a self,
        directive: &str,
        inherit: bool,
    ) -> Option<&'a crate::config::ServerConfigurationDirectiveEntry> {
        for (i, layer) in self.layers.iter().enumerate().rev() {
            if let Some(entry) = layer
                .directives
                .get(directive)
                .and_then(|entries| entries.last())
            {
                return Some(entry);
            }
            if !inherit && i < self.skip_noinherit_from {
                return self.global_fallback_entry(directive);
            }
        }
        None
    }

    /// Entry from global-scope layers only.
    #[inline]
    fn global_fallback_entry<'a>(
        &'a self,
        directive: &str,
    ) -> Option<&'a crate::config::ServerConfigurationDirectiveEntry> {
        self.layers
            .iter()
            .take(self.global_layer_count)
            .rev()
            .find_map(|layer| {
                layer
                    .directives
                    .get(directive)
                    .and_then(|entries| entries.last())
            })
    }

    /// Get the first value for a directive across layers.
    ///
    /// Returns the first argument of the highest-priority matching entry.
    /// When `inherit` is `false`, only the host chain (matched host plus
    /// nested layers) and global-scope layers are checked; less-specific
    /// host layers are skipped.
    #[inline]
    pub fn get_value(
        &self,
        directive: &str,
        inherit: bool,
    ) -> Option<&crate::config::ServerConfigurationValue> {
        for (i, layer) in self.layers.iter().enumerate().rev() {
            if let Some(value) = layer
                .directives
                .get(directive)
                .and_then(|entries| entries.last())
                .and_then(|entry| entry.args.first())
            {
                return Some(value);
            }
            if !inherit && i < self.skip_noinherit_from {
                return self.global_fallback_value(directive);
            }
        }
        None
    }

    /// Get the first value for a directive nested inside a named sub-block.
    ///
    /// Each layer stores its directives as a flat map, so a directive written
    /// inside a block (for example `http { options_allowed_methods }`) is not
    /// visible to [`get_value`](Self::get_value). This helper looks inside
    /// `sub_block` while keeping the same layer precedence and inheritance
    /// rules as `get_value`.
    ///
    /// # Arguments
    ///
    /// * `sub_block` -- Name of the block that holds the directive, for example `http`.
    /// * `directive` -- The directive name to search for inside that block.
    /// * `inherit` -- Same meaning as in [`get_value`](Self::get_value).
    #[inline]
    pub fn get_nested_value(
        &self,
        sub_block: &str,
        directive: &str,
        inherit: bool,
    ) -> Option<&crate::config::ServerConfigurationValue> {
        fn nested<'a>(
            layer: &'a Arc<crate::config::ServerConfigurationBlock>,
            sub_block: &str,
            directive: &str,
        ) -> Option<&'a crate::config::ServerConfigurationValue> {
            layer
                .directives
                .get(sub_block)
                .and_then(|entries| entries.last())
                .and_then(|entry| entry.children.as_ref())
                .and_then(|children| children.directives.get(directive))
                .and_then(|entries| entries.last())
                .and_then(|entry| entry.args.first())
        }

        for (i, layer) in self.layers.iter().enumerate().rev() {
            if let Some(value) = nested(layer, sub_block, directive) {
                return Some(value);
            }
            if !inherit && i < self.skip_noinherit_from {
                return self.layers[..self.global_layer_count]
                    .iter()
                    .rev()
                    .find_map(|layer| nested(layer, sub_block, directive));
            }
        }
        None
    }

    /// Value from global-scope layers only.
    #[inline]
    fn global_fallback_value(
        &self,
        directive: &str,
    ) -> Option<&crate::config::ServerConfigurationValue> {
        self.layers
            .iter()
            .take(self.global_layer_count)
            .rev()
            .find_map(|layer| {
                layer
                    .directives
                    .get(directive)
                    .and_then(|entries| entries.last())
                    .and_then(|entry| entry.args.first())
            })
    }

    /// Get a directive as a boolean flag across layers.
    ///
    /// Returns `true` if the directive is present and its first argument is
    /// a boolean with value `true`, or if the directive is present with no
    /// arguments. Returns `false` if the directive is absent.
    /// When `inherit` is `false`, only the host chain (matched host plus
    /// nested layers) and global-scope layers are checked; less-specific
    /// host layers are skipped.
    #[inline]
    pub fn get_flag(&self, directive: &str, inherit: bool) -> bool {
        for (i, layer) in self.layers.iter().enumerate().rev() {
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
            if !inherit && i < self.skip_noinherit_from {
                return self.global_fallback_flag(directive);
            }
        }
        false
    }

    /// Flag from global-scope layers only.
    #[inline]
    fn global_fallback_flag(&self, directive: &str) -> bool {
        for layer in self.layers.iter().take(self.global_layer_count).rev() {
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
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::LayeredConfiguration;
    use crate::config::{
        ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue,
    };
    use rustc_hash::FxHashMap;
    use std::sync::Arc;

    fn make_block(directives: Vec<(&str, Vec<&str>)>) -> ServerConfigurationBlock {
        let mut map = FxHashMap::default();
        for (name, args) in directives {
            let entry = ServerConfigurationDirectiveEntry {
                args: args
                    .into_iter()
                    .map(|s| ServerConfigurationValue::String(s.into(), None))
                    .collect(),
                children: None,
                span: None,
            };
            map.entry(name.to_string())
                .or_insert_with(Vec::new)
                .push(entry);
        }
        ServerConfigurationBlock {
            directives: Arc::new(map),
            matchers: FxHashMap::default(),
            span: None,
        }
    }

    fn make_nested_block(
        sub_block: &str,
        directives: Vec<(&str, &str)>,
    ) -> ServerConfigurationBlock {
        let children = make_block(
            directives
                .into_iter()
                .map(|(name, value)| (name, vec![value]))
                .collect(),
        );
        let entry = ServerConfigurationDirectiveEntry {
            args: Vec::new(),
            children: Some(children),
            span: None,
        };
        let mut map = FxHashMap::default();
        map.insert(sub_block.to_string(), vec![entry]);
        ServerConfigurationBlock {
            directives: Arc::new(map),
            matchers: FxHashMap::default(),
            span: None,
        }
    }

    #[test]
    fn get_nested_value_finds_directive_inside_sub_block() {
        let block = make_nested_block("http", vec![("timeout", "42s")]);

        let mut layered = LayeredConfiguration::new();
        layered.add_layer(Arc::new(block));

        assert_eq!(
            layered
                .get_nested_value("http", "timeout", true)
                .and_then(|value| value.as_str()),
            Some("42s")
        );
        // A flat lookup cannot see nested directives.
        assert!(layered.get_value("timeout", true).is_none());
    }

    #[test]
    fn get_nested_value_prefers_highest_priority_layer() {
        let global = make_nested_block("http", vec![("timeout", "42s")]);
        let host = make_nested_block("http", vec![("protocols", "h1")]);

        let mut layered = LayeredConfiguration::new();
        layered.add_layer(Arc::new(global));
        layered.mark_end_of_global_layers();
        layered.add_layer(Arc::new(host));
        layered.mark_current_skip_noinherit();

        // The host does not set `timeout`, so the global value applies.
        assert_eq!(
            layered
                .get_nested_value("http", "timeout", false)
                .and_then(|value| value.as_str()),
            Some("42s")
        );
        assert_eq!(
            layered
                .get_nested_value("http", "protocols", false)
                .and_then(|value| value.as_str()),
            Some("h1")
        );
        assert!(layered.get_nested_value("http", "missing", false).is_none());
    }

    #[test]
    fn get_nested_value_higher_priority_layer_overrides() {
        let global = make_nested_block("http", vec![("timeout", "42s")]);
        let host = make_nested_block("http", vec![("timeout", "5s")]);

        let mut layered = LayeredConfiguration::new();
        layered.add_layer(Arc::new(global));
        layered.mark_end_of_global_layers();
        layered.add_layer(Arc::new(host));
        layered.mark_current_skip_noinherit();

        assert_eq!(
            layered
                .get_nested_value("http", "timeout", false)
                .and_then(|value| value.as_str()),
            Some("5s")
        );
    }

    #[test]
    fn get_value_prefers_last_entry_in_highest_priority_layer() {
        let low = make_block(vec![("root", vec!["/srv/low"])]);
        let high = make_block(vec![
            ("root", vec!["/srv/high-initial"]),
            ("root", vec!["/srv/high-final"]),
        ]);

        let mut layered = LayeredConfiguration::new();
        layered.add_layer(Arc::new(low));
        layered.add_layer(Arc::new(high));

        assert_eq!(
            layered
                .get_value("root", true)
                .and_then(|value| value.as_str()),
            Some("/srv/high-final")
        );
    }

    #[test]
    fn get_value_without_inheritance_only_checks_highest_priority_layer() {
        let low = make_block(vec![("root", vec!["/srv/low"])]);
        let high = make_block(vec![]);

        let mut layered = LayeredConfiguration::new();
        layered.add_layer(Arc::new(low));
        layered.add_layer(Arc::new(high));

        assert!(layered.get_value("root", false).is_none());
        assert_eq!(
            layered
                .get_value("root", true)
                .and_then(|value| value.as_str()),
            Some("/srv/low")
        );
    }

    #[test]
    fn noinherit_false_skips_generic_hosts_but_keeps_host_chain() {
        // Simulates resolver layers: [global, wildcard `*`, specific host, location].
        let global = make_block(vec![("global_only", vec!["yes"])]);
        let wildcard = make_block(vec![("proxy", vec!["http://127.0.0.1:3001/"])]);
        let host = make_block(vec![("root", vec!["wwwroot"])]);
        let location = make_block(vec![("index", vec!["index.html"])]);

        let mut layered = LayeredConfiguration::new();
        layered.add_layer(Arc::new(global));
        layered.mark_end_of_global_layers();
        layered.add_layer(Arc::new(wildcard));
        layered.add_layer(Arc::new(host));
        layered.mark_current_skip_noinherit();
        layered.add_layer(Arc::new(location));

        // Host-isolated lookups must not leak the wildcard `proxy` into the named host.
        assert!(layered.get_entries("proxy", false).is_empty());
        assert!(!layered.get_entries("proxy", true).is_empty());
        // Host-chain lookups still see the specific host and its locations.
        assert!(!layered.get_entries("root", false).is_empty());
        assert!(!layered.get_entries("index", false).is_empty());
        // Global-scope defaults still apply when `inherit` is `false`.
        assert!(!layered.get_entries("global_only", false).is_empty());
        assert!(!layered.get_host_chain_entries("root").is_empty());
        assert!(layered.get_host_chain_entries("global_only").is_empty());
        assert!(layered.get_host_chain_entries("proxy").is_empty());
    }
}
