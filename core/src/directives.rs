//! Directive metadata for the `ferron directives` CLI subcommand.
//!
//! Directives are descriptive metadata that the server exposes as JSON for
//! editor support (autocomplete, validation). Modules register their
//! directives in
//! [`ModuleLoader::register_directives`](crate::loader::ModuleLoader::register_directives).
//!
//! # Example
//!
//! ```ignore
//! use ferron_core::directives::{Directive, DirectiveSubblock, DirectiveRegistry};
//!
//! fn register(registry: &mut DirectiveRegistry) {
//!     registry.register(
//!         Directive {
//!             name: "cache",
//!             usage: "cache [bool] | cache { ... }",
//!             description: "Enable or configure response caching.",
//!             applicable_protocols: Some(&["http"]),
//!             global_only: false,
//!             subblock_link: Some(DirectiveSubblock::custom("cache")),
//!         },
//!         DirectiveSubblock::default(),
//!     );
//! }
//! ```

use std::collections::HashMap;

/// A subblock grouping for directives in the `ferron directives` output.
///
/// Directives are organized into subblocks for the JSON output. The
/// [`Default`] variant is the top-level group. Use
/// [`DirectiveSubblock::custom`] for module-specific groups.
#[derive(Default, Clone, Copy, Eq, PartialEq, Hash)]
pub enum DirectiveSubblock {
    /// The default subblock for directives.
    #[default]
    Default,
    /// A custom subblock with the given name.
    Custom(&'static str),
}

impl DirectiveSubblock {
    /// Creates a custom subblock with the given name.
    pub fn custom(name: &'static str) -> Self {
        Self::Custom(name)
    }
}

impl std::fmt::Display for DirectiveSubblock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Default => write!(f, "default"),
            Self::Custom(name) => write!(f, "custom_{}", name),
        }
    }
}

impl serde::Serialize for DirectiveSubblock {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

/// Descriptive metadata for a single directive.
///
/// Used by the `ferron directives` CLI subcommand to produce JSON output
/// for editor integration. Modules register instances via
/// [`DirectiveRegistry::register`].
#[derive(Default, serde::Serialize)]
pub struct Directive {
    /// The name of the directive.
    pub name: &'static str,
    /// The usage of the directive.
    pub usage: &'static str,
    /// The description of the directive.
    pub description: &'static str,
    /// The protocols that the directive is applicable to.
    pub applicable_protocols: Option<&'static [&'static str]>,
    /// Whether the directive can only be used in global blocks.
    pub global_only: bool,
    /// The subblock link of the directive.
    pub subblock_link: Option<DirectiveSubblock>,
}

/// Registry of directive metadata, organized by subblock.
///
/// Modules populate this in
/// [`ModuleLoader::register_directives`](crate::loader::ModuleLoader::register_directives).
/// The server serializes it as JSON for the `ferron directives` subcommand.
#[derive(Default)]
pub struct DirectiveRegistry {
    pub directives: HashMap<DirectiveSubblock, Vec<Directive>>,
}

impl DirectiveRegistry {
    /// Creates a new, empty directive registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a directive under the given subblock.
    ///
    /// Returns `&mut Self` for chaining.
    pub fn register(&mut self, directive: Directive, subblock: DirectiveSubblock) -> &mut Self {
        self.directives.entry(subblock).or_default().push(directive);
        self
    }
}

impl serde::Serialize for DirectiveRegistry {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(self.directives.len()))?;
        for (key, directives) in &self.directives {
            map.serialize_entry(&key.to_string(), directives)?;
        }
        map.end()
    }
}
