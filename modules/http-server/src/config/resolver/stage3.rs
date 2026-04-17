use std::{collections::HashMap, net::IpAddr, sync::Arc};

use ferron_core::config::{
    layer::LayeredConfiguration, ServerConfigurationBlock, ServerConfigurationMatcherExpr,
    ServerConfigurationMatcherOperand,
};
use ferron_http::HttpContext;

use super::super::prepare::{PreparedHostConfigurationBlock, PreparedHostConfigurationErrorConfig};
use super::matcher::{
    evaluate_matcher_condition, evaluate_matcher_conditions, resolve_matcher_operand,
    CompiledMatcherExpr,
};
use super::types::ResolvedLocationPath;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionalGroup {
    pub exprs: Vec<ServerConfigurationMatcherExpr>,
    /// When true, this group must NOT match (used for `if_not` blocks).
    pub negated: bool,
}

/// Compiled conditional group with pre-compiled regex patterns.
type CompiledConditionalGroup = (ConditionalGroup, Vec<CompiledMatcherExpr>);

/// Error configuration scope - composable and extensible
///
/// Supports any combination of IP, hostname, path, and conditional scoping.
/// Conditionals are stored as groups to correctly handle nested `if`/`if_not` blocks.
/// Resolution order: most specific (all fields set) → least specific (global)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorConfigScope {
    pub ip: Option<IpAddr>,
    pub hostname: Option<String>,
    pub path: Option<String>,
    pub conditionals: Vec<ConditionalGroup>,
    pub error_code: Option<u16>, // None = default fallback
}

/// Hashable key for error configuration lookup (excludes conditionals)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ErrorConfigScopeKey {
    pub ip: Option<IpAddr>,
    pub hostname: Option<String>,
    pub path: Option<String>,
    pub error_code: Option<u16>,
}

impl ErrorConfigScope {
    /// Convert to a hashable key (excluding conditionals)
    pub fn to_key(&self) -> ErrorConfigScopeKey {
        ErrorConfigScopeKey {
            ip: self.ip,
            hostname: self.hostname.clone(),
            path: self.path.clone(),
            error_code: self.error_code,
        }
    }
}

impl ErrorConfigScope {
    /// Create a global error code scope
    pub fn global(code: u16) -> Self {
        Self {
            ip: None,
            hostname: None,
            path: None,
            conditionals: Vec::new(),
            error_code: Some(code),
        }
    }

    /// Create an IP-specific error code scope
    pub fn ip(ip: IpAddr, code: u16) -> Self {
        Self {
            ip: Some(ip),
            hostname: None,
            path: None,
            conditionals: Vec::new(),
            error_code: Some(code),
        }
    }

    /// Create a hostname-specific error code scope (supports wildcards like *.example.com)
    pub fn hostname(hostname: impl Into<String>, code: u16) -> Self {
        Self {
            ip: None,
            hostname: Some(hostname.into()),
            path: None,
            conditionals: Vec::new(),
            error_code: Some(code),
        }
    }

    /// Create a path-specific error code scope
    pub fn path(path: impl Into<String>, code: u16) -> Self {
        Self {
            ip: None,
            hostname: None,
            path: Some(path.into()),
            conditionals: Vec::new(),
            error_code: Some(code),
        }
    }

    /// Create IP + hostname combination
    pub fn ip_hostname(ip: IpAddr, hostname: impl Into<String>, code: u16) -> Self {
        Self {
            ip: Some(ip),
            hostname: Some(hostname.into()),
            path: None,
            conditionals: Vec::new(),
            error_code: Some(code),
        }
    }

    /// Create IP + path combination
    pub fn ip_path(ip: IpAddr, path: impl Into<String>, code: u16) -> Self {
        Self {
            ip: Some(ip),
            hostname: None,
            path: Some(path.into()),
            conditionals: Vec::new(),
            error_code: Some(code),
        }
    }

    /// Create hostname + path combination
    pub fn hostname_path(hostname: impl Into<String>, path: impl Into<String>, code: u16) -> Self {
        Self {
            ip: None,
            hostname: Some(hostname.into()),
            path: Some(path.into()),
            conditionals: Vec::new(),
            error_code: Some(code),
        }
    }

    /// Create IP + hostname + path combination (most specific)
    pub fn ip_hostname_path(
        ip: IpAddr,
        hostname: impl Into<String>,
        path: impl Into<String>,
        code: u16,
    ) -> Self {
        Self {
            ip: Some(ip),
            hostname: Some(hostname.into()),
            path: Some(path.into()),
            conditionals: Vec::new(),
            error_code: Some(code),
        }
    }

    /// Create global default scope
    pub fn global_default() -> Self {
        Self {
            ip: None,
            hostname: None,
            path: None,
            conditionals: Vec::new(),
            error_code: None,
        }
    }

    /// Create hostname-specific default scope
    pub fn hostname_default(hostname: impl Into<String>) -> Self {
        Self {
            ip: None,
            hostname: Some(hostname.into()),
            path: None,
            conditionals: Vec::new(),
            error_code: None,
        }
    }

    /// Create IP-specific default scope
    pub fn ip_default(ip: IpAddr) -> Self {
        Self {
            ip: Some(ip),
            hostname: None,
            path: None,
            conditionals: Vec::new(),
            error_code: None,
        }
    }

    /// Create path-specific default scope
    pub fn path_default(path: impl Into<String>) -> Self {
        Self {
            ip: None,
            hostname: None,
            path: Some(path.into()),
            conditionals: Vec::new(),
            error_code: None,
        }
    }

    /// Create IP + hostname default scope
    pub fn ip_hostname_default(ip: IpAddr, hostname: impl Into<String>) -> Self {
        Self {
            ip: Some(ip),
            hostname: Some(hostname.into()),
            path: None,
            conditionals: Vec::new(),
            error_code: None,
        }
    }

    /// Create IP + path default scope
    pub fn ip_path_default(ip: IpAddr, path: impl Into<String>) -> Self {
        Self {
            ip: Some(ip),
            hostname: None,
            path: Some(path.into()),
            conditionals: Vec::new(),
            error_code: None,
        }
    }

    /// Create hostname + path default scope
    pub fn hostname_path_default(hostname: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            ip: None,
            hostname: Some(hostname.into()),
            path: Some(path.into()),
            conditionals: Vec::new(),
            error_code: None,
        }
    }

    /// Create IP + hostname + path default scope
    pub fn ip_hostname_path_default(
        ip: IpAddr,
        hostname: impl Into<String>,
        path: impl Into<String>,
    ) -> Self {
        Self {
            ip: Some(ip),
            hostname: Some(hostname.into()),
            path: Some(path.into()),
            conditionals: Vec::new(),
            error_code: None,
        }
    }
}

/// Stage 3 resolver: Error configuration lookup with scoped support
///
/// Uses a single HashMap with composable ErrorConfigScopeKey keys for non-conditional scopes.
/// Conditional error configs are stored separately and evaluated at resolution time.
/// Supports any combination of IP, hostname, and path scoping.
/// Resolution order: most specific → least specific (global default)
#[derive(Debug, Clone)]
pub struct Stage3ErrorResolver {
    /// Map for non-conditional error configurations keyed by scope
    configs: HashMap<ErrorConfigScopeKey, Arc<PreparedHostConfigurationBlock>>,
    /// Conditional error configurations stored separately for runtime evaluation.
    /// Each entry: (scope, compiled_groups, config, priority)
    conditional_configs: Vec<(
        ErrorConfigScope,
        Vec<CompiledConditionalGroup>,
        Arc<PreparedHostConfigurationBlock>,
        u32,
    )>,
}

impl Stage3ErrorResolver {
    pub fn new() -> Self {
        Self {
            configs: HashMap::new(),
            conditional_configs: Vec::new(),
        }
    }

    /// Register an error configuration for a specific scope
    pub fn register(
        &mut self,
        scope: ErrorConfigScope,
        config: Arc<PreparedHostConfigurationBlock>,
    ) {
        // If scope has conditionals, store in conditional_configs
        if !scope.conditionals.is_empty() {
            let compiled_groups: Result<Vec<_>, _> = scope
                .conditionals
                .iter()
                .map(|group| {
                    let compiled: Result<Vec<_>, _> = group
                        .exprs
                        .iter()
                        .map(|expr| CompiledMatcherExpr::new(expr.clone()))
                        .collect();
                    compiled.map(|c| (group.clone(), c))
                })
                .collect();
            if let Ok(compiled_groups) = compiled_groups {
                let priority = 0u32;
                self.conditional_configs
                    .push((scope, compiled_groups, config, priority));
            }
        } else {
            // Non-conditional, store in regular configs using the key
            self.configs.insert(scope.to_key(), config);
        }
    }

    /// Register an error configuration with conditional groups
    pub fn register_conditional(
        &mut self,
        scope: ErrorConfigScope,
        groups: Vec<ConditionalGroup>,
        priority: u32,
        config: Arc<PreparedHostConfigurationBlock>,
    ) {
        if groups.is_empty() {
            // No conditionals, use regular register
            self.register(scope, config);
            return;
        }

        let compiled_groups: Result<Vec<_>, _> = groups
            .iter()
            .map(|group| {
                let compiled: Result<Vec<_>, _> = group
                    .exprs
                    .iter()
                    .map(|expr| CompiledMatcherExpr::new(expr.clone()))
                    .collect();
                compiled.map(|c| (group.clone(), c))
            })
            .collect();
        if let Ok(compiled_groups) = compiled_groups {
            self.conditional_configs
                .push((scope, compiled_groups, config, priority));
        }
    }

    /// Register a global error configuration
    pub fn register_error(&mut self, code: u16, config: Arc<PreparedHostConfigurationBlock>) {
        self.configs
            .insert(ErrorConfigScope::global(code).to_key(), config);
    }

    /// Register a hostname-specific error configuration (supports wildcards like *.example.com)
    pub fn register_hostname_error(
        &mut self,
        hostname: &str,
        code: u16,
        config: Arc<PreparedHostConfigurationBlock>,
    ) {
        self.configs
            .insert(ErrorConfigScope::hostname(hostname, code).to_key(), config);
    }

    /// Register an IP-specific error configuration
    pub fn register_ip_error(
        &mut self,
        ip: IpAddr,
        code: u16,
        config: Arc<PreparedHostConfigurationBlock>,
    ) {
        self.configs
            .insert(ErrorConfigScope::ip(ip, code).to_key(), config);
    }

    /// Register a path-specific error configuration
    pub fn register_path_error(
        &mut self,
        path_prefix: &str,
        code: u16,
        config: Arc<PreparedHostConfigurationBlock>,
    ) {
        self.configs
            .insert(ErrorConfigScope::path(path_prefix, code).to_key(), config);
    }

    /// Register an IP + hostname combination error configuration
    pub fn register_ip_hostname_error(
        &mut self,
        ip: IpAddr,
        hostname: &str,
        code: u16,
        config: Arc<PreparedHostConfigurationBlock>,
    ) {
        self.configs.insert(
            ErrorConfigScope::ip_hostname(ip, hostname, code).to_key(),
            config,
        );
    }

    /// Register a hostname + path combination error configuration
    pub fn register_hostname_path_error(
        &mut self,
        hostname: &str,
        path: &str,
        code: u16,
        config: Arc<PreparedHostConfigurationBlock>,
    ) {
        self.configs.insert(
            ErrorConfigScope::hostname_path(hostname, path, code).to_key(),
            config,
        );
    }

    /// Register an IP + hostname + path combination error configuration (most specific)
    pub fn register_ip_hostname_path_error(
        &mut self,
        ip: IpAddr,
        hostname: &str,
        path: &str,
        code: u16,
        config: Arc<PreparedHostConfigurationBlock>,
    ) {
        self.configs.insert(
            ErrorConfigScope::ip_hostname_path(ip, hostname, path, code).to_key(),
            config,
        );
    }

    /// Set the default error configuration (global fallback)
    pub fn set_default(&mut self, config: Arc<PreparedHostConfigurationBlock>) {
        self.configs
            .insert(ErrorConfigScope::global_default().to_key(), config);
    }

    /// Set a hostname-specific default error configuration
    pub fn set_hostname_default(
        &mut self,
        hostname: &str,
        config: Arc<PreparedHostConfigurationBlock>,
    ) {
        self.configs.insert(
            ErrorConfigScope::hostname_default(hostname).to_key(),
            config,
        );
    }

    /// Set an IP-specific default error configuration
    pub fn set_ip_default(&mut self, ip: IpAddr, config: Arc<PreparedHostConfigurationBlock>) {
        self.configs
            .insert(ErrorConfigScope::ip_default(ip).to_key(), config);
    }

    /// Set a path-specific default error configuration
    pub fn set_path_default(
        &mut self,
        path_prefix: &str,
        config: Arc<PreparedHostConfigurationBlock>,
    ) {
        self.configs
            .insert(ErrorConfigScope::path_default(path_prefix).to_key(), config);
    }

    /// Set an IP + hostname default error configuration
    pub fn set_ip_hostname_default(
        &mut self,
        ip: IpAddr,
        hostname: &str,
        config: Arc<PreparedHostConfigurationBlock>,
    ) {
        self.configs.insert(
            ErrorConfigScope::ip_hostname_default(ip, hostname).to_key(),
            config,
        );
    }

    /// Set a hostname + path default error configuration
    pub fn set_hostname_path_default(
        &mut self,
        hostname: &str,
        path: &str,
        config: Arc<PreparedHostConfigurationBlock>,
    ) {
        self.configs.insert(
            ErrorConfigScope::hostname_path_default(hostname, path).to_key(),
            config,
        );
    }

    /// Set an IP + hostname + path default error configuration
    pub fn set_ip_hostname_path_default(
        &mut self,
        ip: IpAddr,
        hostname: &str,
        path: &str,
        config: Arc<PreparedHostConfigurationBlock>,
    ) {
        self.configs.insert(
            ErrorConfigScope::ip_hostname_path_default(ip, hostname, path).to_key(),
            config,
        );
    }

    /// Resolve error configuration by code (global only - legacy method)
    pub fn resolve(
        &self,
        error_code: u16,
        location_path: &mut ResolvedLocationPath,
    ) -> Option<Arc<PreparedHostConfigurationBlock>> {
        location_path.error_key = Some(error_code);
        self.configs
            .get(&ErrorConfigScope::global(error_code).to_key())
            .cloned()
            .or_else(|| {
                self.configs
                    .get(&ErrorConfigScope::global_default().to_key())
                    .cloned()
            })
    }

    /// Generate all possible scopes from most specific to least specific
    fn generate_scopes(
        error_code: u16,
        hostname: Option<&str>,
        ip: Option<IpAddr>,
        path_segments: Option<&[String]>,
    ) -> Vec<ErrorConfigScope> {
        let mut scopes = Vec::with_capacity(8);
        let path_str = path_segments.map(|s| s.join("/"));

        // 1. Most specific: IP + Hostname + Path
        if let (Some(ip), Some(hostname), Some(path)) = (ip, hostname, &path_str) {
            scopes.push(ErrorConfigScope::ip_hostname_path(
                ip,
                hostname,
                path.clone(),
                error_code,
            ));
        }
        // 2. IP + Hostname
        if let (Some(ip), Some(hostname)) = (ip, hostname) {
            scopes.push(ErrorConfigScope::ip_hostname(ip, hostname, error_code));
        }
        // 3. Hostname + Path
        if let (Some(hostname), Some(path)) = (hostname, &path_str) {
            scopes.push(ErrorConfigScope::hostname_path(
                hostname,
                path.clone(),
                error_code,
            ));
        }
        // 4. IP + Path
        if let (Some(ip), Some(path)) = (ip, &path_str) {
            scopes.push(ErrorConfigScope::ip_path(ip, path.clone(), error_code));
        }
        // 5. Path only
        if let Some(path) = &path_str {
            scopes.push(ErrorConfigScope::path(path.clone(), error_code));
        }
        // 6. Hostname only
        if let Some(hostname) = hostname {
            scopes.push(ErrorConfigScope::hostname(hostname, error_code));
        }
        // 7. IP only
        if let Some(ip) = ip {
            scopes.push(ErrorConfigScope::ip(ip, error_code));
        }
        // 8. Global (least specific)
        scopes.push(ErrorConfigScope::global(error_code));
        scopes
    }

    /// Generate all default scopes from most specific to least specific
    fn generate_default_scopes(
        hostname: Option<&str>,
        ip: Option<IpAddr>,
        path_segments: Option<&[String]>,
    ) -> Vec<ErrorConfigScope> {
        let mut scopes = Vec::with_capacity(8);
        let path_str = path_segments.map(|s| s.join("/"));

        // 1. Most specific: IP + Hostname + Path default
        if let (Some(ip), Some(hostname), Some(path)) = (ip, hostname, &path_str) {
            scopes.push(ErrorConfigScope::ip_hostname_path_default(
                ip,
                hostname,
                path.clone(),
            ));
        }
        // 2. IP + Hostname default
        if let (Some(ip), Some(hostname)) = (ip, hostname) {
            scopes.push(ErrorConfigScope::ip_hostname_default(ip, hostname));
        }
        // 3. Hostname + Path default
        if let (Some(hostname), Some(path)) = (hostname, &path_str) {
            scopes.push(ErrorConfigScope::hostname_path_default(
                hostname,
                path.clone(),
            ));
        }
        // 4. IP + Path default
        if let (Some(ip), Some(path)) = (ip, &path_str) {
            scopes.push(ErrorConfigScope::ip_path_default(ip, path.clone()));
        }
        // 5. Path default
        if let Some(path) = &path_str {
            scopes.push(ErrorConfigScope::path_default(path.clone()));
        }
        // 6. Hostname default
        if let Some(hostname) = hostname {
            scopes.push(ErrorConfigScope::hostname_default(hostname));
        }
        // 7. IP default
        if let Some(ip) = ip {
            scopes.push(ErrorConfigScope::ip_default(ip));
        }
        // 8. Global default (least specific)
        scopes.push(ErrorConfigScope::global_default());
        scopes
    }

    /// Evaluate conditional expression groups with proper negation handling.
    /// Each group must match (for `if`) or must NOT match (for `if_not`).
    /// All groups use AND logic — every group's requirement must be satisfied.
    fn evaluate_condition_groups(
        &self,
        compiled_groups: &[CompiledConditionalGroup],
        ctx: &HttpContext,
    ) -> bool {
        compiled_groups.iter().all(|(group, compiled_exprs)| {
            let matches = evaluate_matcher_conditions(compiled_exprs, ctx);
            if group.negated {
                !matches
            } else {
                matches
            }
        })
    }

    /// Resolve error configuration with scoped lookup
    ///
    /// Resolution order (most specific to least specific):
    /// 1. IP + Hostname + Path + ErrorCode
    /// 2. IP + Hostname + ErrorCode
    /// 3. Hostname + Path + ErrorCode
    /// 4. IP + Path + ErrorCode
    /// 5. Path + ErrorCode
    /// 6. Hostname + ErrorCode
    /// 7. IP + ErrorCode
    /// 8. Global ErrorCode
    /// 9-16. Same combinations for defaults (error_code = None)
    ///
    /// Additionally checks conditional error configs that match the current variables.
    #[allow(clippy::doc_lazy_continuation)]
    pub fn resolve_scoped(
        &self,
        error_code: u16,
        hostname: Option<&str>,
        ip: Option<IpAddr>,
        path_segments: Option<&[String]>,
        ctx: &HttpContext,
        location_path: &mut ResolvedLocationPath,
    ) -> Option<Arc<PreparedHostConfigurationBlock>> {
        location_path.error_key = Some(error_code);

        // Pre-compute the joined path string once to avoid repeated allocations
        let path_str = path_segments.map(|s| {
            let mut joined = String::with_capacity(s.iter().map(|s| s.len() + 1).sum::<usize>());
            for (i, segment) in s.iter().enumerate() {
                if i > 0 {
                    joined.push('/');
                }
                joined.push_str(segment);
            }
            joined
        });
        let path_str_ref = path_str.as_deref();

        // First, try conditional configs (sorted by priority, highest first)
        let mut matching_conditionals: Vec<_> = self
            .conditional_configs
            .iter()
            .filter(|(scope, compiled_groups, _, _)| {
                // Check if scope matches (ignoring conditionals in the key)
                Self::scope_matches(scope, hostname, ip, path_str_ref, error_code)
                    && self.evaluate_condition_groups(compiled_groups, ctx)
            })
            .map(|(_, _, config, priority)| (*priority, Arc::clone(config)))
            .collect();

        // Sort by priority (highest first)
        matching_conditionals.sort_by_key(|b| std::cmp::Reverse(b.0));

        // Return highest priority matching conditional config
        if let Some((_, config)) = matching_conditionals.into_iter().next() {
            return Some(config);
        }

        // Generate all possible scopes from most to least specific
        let scopes = Self::generate_scopes(error_code, hostname, ip, path_segments);

        // Try each scope in order
        for scope in scopes {
            if let Some(config) = self.configs.get(&scope.to_key()) {
                return Some(config.clone());
            }
        }

        // Try defaults (same order, but error_code = None)
        let default_scopes = Self::generate_default_scopes(hostname, ip, path_segments);
        for scope in default_scopes {
            if let Some(config) = self.configs.get(&scope.to_key()) {
                return Some(config.clone());
            }
        }

        None
    }

    /// Check if an error config scope matches the given resolution context
    fn scope_matches(
        scope: &ErrorConfigScope,
        hostname: Option<&str>,
        ip: Option<IpAddr>,
        path_str: Option<&str>,
        error_code: u16,
    ) -> bool {
        // Check error code matches (or is None for defaults)
        if let Some(code) = scope.error_code {
            if code != error_code {
                return false;
            }
        }

        // Check IP matches
        if let Some(scope_ip) = scope.ip {
            if ip != Some(scope_ip) {
                return false;
            }
        }

        // Check hostname matches
        if let Some(ref scope_hostname) = scope.hostname {
            if hostname != Some(scope_hostname.as_str()) {
                return false;
            }
        }

        // Check path matches
        if let Some(ref scope_path) = scope.path {
            if path_str != Some(scope_path.as_str()) {
                return false;
            }
        }

        true
    }

    /// Resolve default error configuration with scoped lookup
    pub fn resolve_default_scoped(
        &self,
        hostname: Option<&str>,
        ip: Option<IpAddr>,
        path_segments: Option<&[String]>,
        ctx: &HttpContext,
    ) -> Option<Arc<PreparedHostConfigurationBlock>> {
        // Pre-compute the joined path string once to avoid repeated allocations
        let path_str = path_segments.map(|s| {
            let mut joined = String::with_capacity(s.iter().map(|s| s.len() + 1).sum::<usize>());
            for (i, segment) in s.iter().enumerate() {
                if i > 0 {
                    joined.push('/');
                }
                joined.push_str(segment);
            }
            joined
        });
        let path_str_ref = path_str.as_deref();

        // First, try conditional configs with error_code = None (defaults)
        let mut matching_conditionals: Vec<_> = self
            .conditional_configs
            .iter()
            .filter(|(scope, compiled_groups, _, _)| {
                // Check if scope matches for defaults (error_code = None)
                Self::scope_matches_for_default(scope, hostname, ip, path_str_ref)
                    && self.evaluate_condition_groups(compiled_groups, ctx)
            })
            .map(|(_, _, config, priority)| (*priority, Arc::clone(config)))
            .collect();

        // Sort by priority (highest first)
        matching_conditionals.sort_by_key(|b| std::cmp::Reverse(b.0));

        // Return highest priority matching conditional config
        if let Some((_, config)) = matching_conditionals.into_iter().next() {
            return Some(config);
        }

        // Fall back to non-conditional defaults
        let default_scopes = Self::generate_default_scopes(hostname, ip, path_segments);
        for scope in default_scopes {
            if let Some(config) = self.configs.get(&scope.to_key()) {
                return Some(config.clone());
            }
        }
        None
    }

    /// Check if an error config scope matches for default resolution (error_code = None)
    fn scope_matches_for_default(
        scope: &ErrorConfigScope,
        hostname: Option<&str>,
        ip: Option<IpAddr>,
        path_str: Option<&str>,
    ) -> bool {
        // For defaults, error_code should be None
        if scope.error_code.is_some() {
            return false;
        }

        // Check IP matches
        if let Some(scope_ip) = scope.ip {
            if ip != Some(scope_ip) {
                return false;
            }
        }

        // Check hostname matches
        if let Some(ref scope_hostname) = scope.hostname {
            if hostname != Some(scope_hostname.as_str()) {
                return false;
            }
        }

        // Check path matches
        if let Some(ref scope_path) = scope.path {
            if path_str != Some(scope_path.as_str()) {
                return false;
            }
        }

        true
    }

    /// Resolve error configuration and create a layered configuration (global only - legacy)
    ///
    /// # Arguments
    /// * `error_code` - Error code to resolve
    /// * `base_config` - Optional base layered configuration to add layers to
    pub fn resolve_layered(
        &self,
        error_code: u16,
        base_config: Option<LayeredConfiguration>,
    ) -> (LayeredConfiguration, ResolvedLocationPath) {
        let mut location_path = ResolvedLocationPath::new();
        let mut layered_config = base_config.unwrap_or_default();

        if let Some(config) = self.resolve(error_code, &mut location_path) {
            // Clone the Arc (cheap - just increments ref count)
            let block = ServerConfigurationBlock {
                directives: Arc::clone(&config.directives),
                matchers: HashMap::new(),
                span: None,
            };
            layered_config.add_layer(Arc::new(block));
        }

        (layered_config, location_path)
    }

    /// Resolve error configuration with scoped lookup and create a layered configuration
    ///
    /// This method properly chains Stage 3 on top of Stage 2's base configuration.
    ///
    /// # Arguments
    /// * `error_code` - Error code to resolve
    /// * `hostname` - Optional hostname for scoped lookup
    /// * `ip` - Optional IP for scoped lookup
    /// * `path_segments` - Optional path segments for scoped lookup
    /// * `ctx` - HTTP context for conditional evaluation
    /// * `base_config` - Base layered configuration from Stage 2
    pub fn resolve_layered_scoped(
        &self,
        error_code: u16,
        hostname: Option<&str>,
        ip: Option<IpAddr>,
        path_segments: Option<&[String]>,
        ctx: &HttpContext,
        base_config: Option<LayeredConfiguration>,
    ) -> (LayeredConfiguration, ResolvedLocationPath) {
        let mut location_path = ResolvedLocationPath::new();
        let mut layered_config = base_config.unwrap_or_default();

        // Try to resolve specific error code first
        let error_config = self.resolve_scoped(
            error_code,
            hostname,
            ip,
            path_segments,
            ctx,
            &mut location_path,
        );

        // If no specific error config found, try default (also scoped)
        let error_config =
            error_config.or_else(|| self.resolve_default_scoped(hostname, ip, path_segments, ctx));

        if let Some(config) = error_config {
            let block = ServerConfigurationBlock {
                directives: Arc::clone(&config.directives),
                matchers: HashMap::new(),
                span: None,
            };
            layered_config.add_layer(Arc::new(block));
        }

        (layered_config, location_path)
    }

    /// Build from PreparedHostConfigurationBlock error configs
    pub fn from_error_configs(error_configs: &[PreparedHostConfigurationErrorConfig]) -> Self {
        let mut resolver = Self::new();

        for error_config in error_configs {
            let config = Arc::new(error_config.config.clone());
            if let Some(code) = error_config.error_code {
                resolver.register_error(code, config);
            } else {
                resolver.set_default(config);
            }
        }

        resolver
    }
}

impl Default for Stage3ErrorResolver {
    fn default() -> Self {
        Self::new()
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
    fn test_stage3_error_resolver() {
        let mut resolver = Stage3ErrorResolver::new();

        let config = Arc::new(create_test_block());
        resolver.register_error(404, config);

        let mut path = ResolvedLocationPath::new();
        let result = resolver.resolve(404, &mut path);

        assert!(result.is_some());
        assert_eq!(path.error_key, Some(404));
    }

    #[test]
    fn test_stage3_layered_resolution() {
        let mut resolver = Stage3ErrorResolver::new();

        let config = Arc::new(create_test_block());
        resolver.register_error(500, Arc::clone(&config));
        resolver.register_error(404, config);

        let (layered_config, path) = resolver.resolve_layered(404, None);

        assert_eq!(path.error_key, Some(404));
        assert_eq!(layered_config.layers.len(), 1);
    }
}
