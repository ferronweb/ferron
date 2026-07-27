use std::collections::HashMap;

/// Represents a subblock for directives for `ferron directives`.
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

/// Represents a directive for `ferron directives`.
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

/// Represents a registry of directives for `ferron directives`.
#[derive(Default)]
pub struct DirectiveRegistry {
    pub directives: HashMap<DirectiveSubblock, Vec<Directive>>,
}

impl DirectiveRegistry {
    /// Creates a new, empty directive registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a directive with the given name, usage, description, and subblock.
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
