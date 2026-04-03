use std::{collections::HashMap, fmt, net::IpAddr};

use ferron_core::config::{layer::LayeredConfiguration, ServerConfigurationMatcherExpr};
use ferron_http::HttpRequest;

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
}

impl fmt::Display for ResolvedLocationPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
            write!(f, "root")
        } else {
            write!(f, "{}", parts.join(" > "))
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
