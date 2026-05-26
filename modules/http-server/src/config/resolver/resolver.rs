use std::{collections::HashMap, net::IpAddr, sync::Arc};

use ferron_core::config::{
    layer::LayeredConfiguration, ServerConfigurationBlock, ServerConfigurationMatcherExpr,
};
use ferron_http::HttpContext;

use super::super::prepare::{
    PreparedConfiguration, PreparedHostConfigurationBlock, PreparedHostConfigurationErrorConfig,
    PreparedHostConfigurationMatch, PreparedHostConfigurationMatcher,
};
use super::tree::*;
use super::types::{ResolutionResult, ResolvedLocationPath};

#[derive(Debug, Clone, Default)]
struct ErrorHandlerStatusLookup<T> {
    catchall_values: Vec<T>,
    status_code_values: HashMap<u16, Vec<T>>,
}

impl<T> ErrorHandlerStatusLookup<T> {
    #[inline]
    fn new() -> Self {
        Self {
            catchall_values: Vec::new(),
            status_code_values: HashMap::new(),
        }
    }

    #[inline]
    fn get(&self, status_code: u16) -> Vec<&T> {
        let mut values = Vec::new();

        for value in &self.catchall_values {
            values.push(value);
        }

        if let Some(exact_values) = self.status_code_values.get(&status_code) {
            for value in exact_values {
                values.push(value);
            }
        }

        values
    }

    #[inline]
    fn insert(&mut self, status_code: Option<u16>, value: T) {
        if let Some(code) = status_code {
            self.status_code_values.entry(code).or_default().push(value);
        } else {
            self.catchall_values.push(value);
        }
    }
}

#[derive(Debug)]
struct CompiledBlock {
    layer: Arc<ServerConfigurationBlock>,
    branches: HostLookupTree<Arc<CompiledBlock>>,
    error_handlers: ErrorHandlerStatusLookup<Arc<CompiledBlock>>,
}

#[derive(Debug, Clone)]
struct ResolvedBlockMatch {
    block: Arc<CompiledBlock>,
    matched_keys: Vec<HostLookupKey>,
    consumed_input_len: usize,
}

#[derive(Debug)]
struct MatchedScope {
    block: Arc<CompiledBlock>,
    remaining_path_segments: Vec<String>,
}

struct BaseResolution {
    configuration: LayeredConfiguration,
    location_path: ResolvedLocationPath,
    matched_scopes: Vec<MatchedScope>,
}

pub struct ThreeStageResolver {
    global: Option<Arc<ServerConfigurationBlock>>,
    generic_hosts: HostLookupTree<Arc<CompiledBlock>>,
    scoped_hosts: HostLookupTree<Arc<CompiledBlock>>,
}

impl ThreeStageResolver {
    #[inline]
    pub fn new() -> Self {
        Self {
            global: None,
            generic_hosts: HostLookupTree::new(),
            scoped_hosts: HostLookupTree::new(),
        }
    }

    #[inline]
    pub fn from_prepared(prepared: PreparedConfiguration) -> Self {
        let mut resolver = Self::new();

        for (ip_opt, hosts) in prepared {
            if let Some(default_host) = &hosts.default_host {
                let compiled = Self::compile_block(Arc::clone(default_host));
                resolver.insert_host(ip_opt, None, compiled);
            }

            for (hostname, block) in &hosts.named_hosts {
                let compiled = Self::compile_block(Arc::clone(block));
                resolver.insert_host(ip_opt, Some(hostname.as_str()), compiled);
            }
        }

        resolver
    }

    #[inline]
    pub fn from_prepared_with_global(
        prepared: PreparedConfiguration,
        global: Arc<ServerConfigurationBlock>,
    ) -> Self {
        let mut resolver = Self::from_prepared(prepared);
        resolver.global = Some(global);
        resolver
    }

    #[inline]
    pub fn resolve(
        &self,
        ip: IpAddr,
        hostname: &str,
        path: &str,
        ctx: &HttpContext,
    ) -> Option<ResolutionResult> {
        let base = self.resolve_base(ip, hostname, path, ctx)?;
        Some(ResolutionResult::new(
            base.configuration,
            base.location_path,
        ))
    }

    #[inline]
    pub fn resolve_error_scoped(
        &self,
        ip: IpAddr,
        hostname: &str,
        path: &str,
        error_code: u16,
        ctx: &HttpContext,
    ) -> Option<ResolutionResult> {
        let mut base = self.resolve_base(ip, hostname, path, ctx)?;
        base.location_path.error_key = Some(error_code);

        for scope in &base.matched_scopes {
            let remaining_path_keys =
                Self::path_lookup_key_from_segments(&scope.remaining_path_segments);
            for handler in scope.block.error_handlers.get(error_code) {
                base.configuration.add_layer(Arc::clone(&handler.layer));
                Self::apply_nested_layers(
                    handler,
                    &remaining_path_keys,
                    &scope.remaining_path_segments,
                    ctx,
                    &mut base.configuration,
                );
            }
        }

        Some(ResolutionResult::new(
            base.configuration,
            base.location_path,
        ))
    }

    #[inline]
    pub fn global(&self) -> Option<Arc<ServerConfigurationBlock>> {
        self.global.clone()
    }

    #[inline]
    fn insert_host(
        &mut self,
        ip: Option<IpAddr>,
        hostname: Option<&str>,
        block: Arc<CompiledBlock>,
    ) {
        match ip {
            Some(ip) => {
                let key = Self::scoped_host_lookup_key(ip, hostname);
                self.scoped_hosts.insert_node(key).replace(block);
            }
            None => {
                let key = Self::generic_host_lookup_key(hostname);
                self.generic_hosts.insert_node(key).replace(block);
            }
        }
    }

    #[inline]
    fn generic_host_lookup_key(hostname: Option<&str>) -> Vec<HostLookupKey> {
        let mut key = Vec::new();

        if let Some(hostname) = hostname {
            key.extend(Self::hostname_lookup_keys(hostname));
        }

        key
    }

    #[inline]
    fn scoped_host_lookup_key(ip: IpAddr, hostname: Option<&str>) -> Vec<HostLookupKey> {
        let mut key = Self::ip_lookup_keys(ip);

        if let Some(hostname) = hostname {
            key.extend(Self::hostname_lookup_keys(hostname));
        }

        key
    }

    #[inline]
    fn hostname_lookup_keys(hostname: &str) -> Vec<HostLookupKey> {
        let mut key = Vec::new();

        for segment in hostname
            .split('.')
            .rev()
            .filter(|segment| !segment.is_empty())
        {
            if segment == "*" {
                key.push(HostLookupKey::HostDomainLevelWildcard);
            } else {
                key.push(HostLookupKey::HostDomainLevel(segment.to_string()));
            }
        }

        if !key.is_empty() {
            key.push(HostLookupKey::HostnameEnd);
        }

        key
    }

    #[inline]
    fn request_hostname_lookup_key(hostname: &str) -> Vec<HostLookupKey> {
        Self::hostname_lookup_keys(hostname)
    }

    #[inline]
    fn ip_lookup_keys(ip: IpAddr) -> Vec<HostLookupKey> {
        if ip.is_loopback() {
            return vec![HostLookupKey::IsLoopback];
        }

        match ip {
            IpAddr::V4(ipv4) => ipv4
                .octets()
                .into_iter()
                .map(HostLookupKey::IPv4Octet)
                .collect(),
            IpAddr::V6(ipv6) => ipv6
                .octets()
                .into_iter()
                .map(HostLookupKey::IPv6Octet)
                .collect(),
        }
    }

    #[inline]
    fn path_lookup_key(path: &str) -> Vec<HostLookupKey> {
        let mut key = Vec::new();
        let mut is_first = true;

        for segment in path.split('/') {
            if is_first || !segment.is_empty() {
                key.push(HostLookupKey::LocationSegment(segment.to_string()));
            }
            is_first = false;
        }

        if key.is_empty() {
            key.push(HostLookupKey::LocationSegment(String::new()));
        }

        key
    }

    #[inline]
    fn path_lookup_key_from_segments(segments: &[String]) -> Vec<HostLookupKey> {
        let mut key = vec![HostLookupKey::LocationSegment(String::new())];
        key.extend(segments.iter().cloned().map(HostLookupKey::LocationSegment));
        key
    }

    #[inline]
    fn split_path_segments(path: &str) -> Vec<String> {
        path.trim_start_matches('/')
            .split('/')
            .filter(|segment| !segment.is_empty())
            .map(str::to_string)
            .collect()
    }

    #[inline]
    fn compile_block(block: Arc<PreparedHostConfigurationBlock>) -> Arc<CompiledBlock> {
        let mut branches = HostLookupTree::new();

        for matcher in &block.matches {
            if let Some((branch_key, child_block)) = Self::compile_match_branch(matcher) {
                branches.insert_node(branch_key).replace(child_block);
            }
        }

        let mut error_handlers = ErrorHandlerStatusLookup::new();
        for error_config in &block.error_config {
            error_handlers.insert(
                error_config.error_code,
                Self::compile_error_block(error_config),
            );
        }

        Arc::new(CompiledBlock {
            layer: Arc::new(ServerConfigurationBlock {
                directives: Arc::clone(&block.directives),
                matchers: HashMap::new(),
                span: None,
            }),
            branches,
            error_handlers,
        })
    }

    #[inline]
    fn compile_match_branch(
        matcher: &PreparedHostConfigurationMatch,
    ) -> Option<(Vec<HostLookupKey>, Arc<CompiledBlock>)> {
        let child_block = Self::compile_block(Arc::clone(&matcher.config));

        match &matcher.matcher {
            PreparedHostConfigurationMatcher::Location(path) => {
                Some((Self::path_lookup_key(path), child_block))
            }
            PreparedHostConfigurationMatcher::IfConditional(exprs) => {
                let key = HostLookupKey::Conditional(ConditionalLookupKey {
                    exprs: exprs.clone(),
                    negated: false,
                });
                PredicateMatcher::from_key(&key)?;
                Some((vec![key], child_block))
            }
            PreparedHostConfigurationMatcher::IfNotConditional(exprs) => {
                let key = HostLookupKey::Conditional(ConditionalLookupKey {
                    exprs: exprs.clone(),
                    negated: true,
                });
                PredicateMatcher::from_key(&key)?;
                Some((vec![key], child_block))
            }
        }
    }

    #[inline]
    fn compile_error_block(
        error_config: &PreparedHostConfigurationErrorConfig,
    ) -> Arc<CompiledBlock> {
        Self::compile_block(Arc::new(error_config.config.clone()))
    }

    #[inline]
    fn resolve_base(
        &self,
        ip: IpAddr,
        hostname: &str,
        path: &str,
        ctx: &HttpContext,
    ) -> Option<BaseResolution> {
        let host_matches = self.resolve_host_matches(ip, hostname, ctx);
        if host_matches.is_empty() {
            return None;
        }

        let request_path_keys = Self::path_lookup_key(path);
        let request_path_segments = Self::split_path_segments(path);

        let mut configuration = LayeredConfiguration::new();
        if let Some(global) = self.global.clone() {
            configuration.add_layer(global);
        }

        let mut matched_scopes = Vec::new();
        let mut matched_path_segments = Vec::new();
        let mut matched_conditionals = Vec::new();

        for host_match in &host_matches {
            configuration.add_layer(Arc::clone(&host_match.block.layer));
            matched_scopes.push(MatchedScope {
                block: Arc::clone(&host_match.block),
                remaining_path_segments: request_path_segments.clone(),
            });

            Self::resolve_block(
                &host_match.block,
                &request_path_keys,
                &request_path_segments,
                ctx,
                &[],
                &mut configuration,
                &mut matched_scopes,
                &mut matched_path_segments,
                &mut matched_conditionals,
            );
        }

        let hostname_segments = host_matches
            .last()
            .map(|host_match| Self::matched_hostname_segments(&host_match.matched_keys))
            .unwrap_or_default();

        Some(BaseResolution {
            configuration,
            location_path: ResolvedLocationPath {
                ip: Some(ip),
                hostname_segments,
                path_segments: matched_path_segments,
                conditionals: matched_conditionals,
                error_key: None,
            },
            matched_scopes,
        })
    }

    #[inline]
    fn resolve_host_matches(
        &self,
        ip: IpAddr,
        hostname: &str,
        ctx: &HttpContext,
    ) -> Vec<ResolvedBlockMatch> {
        let generic_request_key = Self::request_hostname_lookup_key(hostname);
        let scoped_request_key = Self::scoped_host_lookup_key(ip, Some(hostname));

        let generic_matches = self
            .generic_hosts
            .get(&generic_request_key, ctx)
            .into_iter()
            .map(Self::owned_lookup_match)
            .collect::<Vec<_>>();
        let scoped_matches = self
            .scoped_hosts
            .get(&scoped_request_key, ctx)
            .into_iter()
            .map(Self::owned_lookup_match)
            .collect::<Vec<_>>();

        let mut matches = Vec::new();

        if let Some(default_match) = generic_matches
            .iter()
            .find(|matched| matched.matched_keys.is_empty())
        {
            matches.push(default_match.clone());
        }

        matches.extend(
            scoped_matches
                .iter()
                .filter(|matched| !Self::has_hostname_keys(&matched.matched_keys))
                .cloned(),
        );
        matches.extend(
            generic_matches
                .iter()
                .filter(|matched| Self::has_hostname_keys(&matched.matched_keys))
                .cloned(),
        );
        matches.extend(
            scoped_matches
                .iter()
                .filter(|matched| Self::has_hostname_keys(&matched.matched_keys))
                .cloned(),
        );

        matches
    }

    #[inline]
    fn owned_lookup_match(
        match_result: HostLookupMatch<'_, Arc<CompiledBlock>>,
    ) -> ResolvedBlockMatch {
        ResolvedBlockMatch {
            block: Arc::clone(match_result.value),
            matched_keys: match_result.matched_keys,
            consumed_input_len: match_result.consumed_input_len,
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[inline]
    fn resolve_block(
        block: &Arc<CompiledBlock>,
        request_path_keys: &[HostLookupKey],
        request_path_segments: &[String],
        ctx: &HttpContext,
        matched_path_prefix: &[String],
        configuration: &mut LayeredConfiguration,
        matched_scopes: &mut Vec<MatchedScope>,
        best_path_segments: &mut Vec<String>,
        matched_conditionals: &mut Vec<ServerConfigurationMatcherExpr>,
    ) {
        let matches = block
            .branches
            .get(request_path_keys, ctx)
            .into_iter()
            .map(Self::owned_lookup_match)
            .collect::<Vec<_>>();

        for branch_match in matches {
            configuration.add_layer(Arc::clone(&branch_match.block.layer));

            let mut full_path_segments = matched_path_prefix.to_vec();
            full_path_segments.extend(Self::matched_location_segments(&branch_match.matched_keys));
            if full_path_segments.len() >= best_path_segments.len() {
                *best_path_segments = full_path_segments.clone();
            }

            matched_conditionals.extend(Self::matched_conditionals(&branch_match.matched_keys));

            let consumed_path_segments =
                Self::consumed_location_segments(&branch_match.matched_keys);
            let remaining_path_keys = &request_path_keys[branch_match.consumed_input_len..];
            let remaining_path_segments =
                &request_path_segments[consumed_path_segments.min(request_path_segments.len())..];

            matched_scopes.push(MatchedScope {
                block: Arc::clone(&branch_match.block),
                remaining_path_segments: remaining_path_segments.to_vec(),
            });

            Self::resolve_block(
                &branch_match.block,
                remaining_path_keys,
                remaining_path_segments,
                ctx,
                &full_path_segments,
                configuration,
                matched_scopes,
                best_path_segments,
                matched_conditionals,
            );
        }
    }

    #[inline]
    fn apply_nested_layers(
        block: &Arc<CompiledBlock>,
        request_path_keys: &[HostLookupKey],
        request_path_segments: &[String],
        ctx: &HttpContext,
        configuration: &mut LayeredConfiguration,
    ) {
        let matches = block
            .branches
            .get(request_path_keys, ctx)
            .into_iter()
            .map(Self::owned_lookup_match)
            .collect::<Vec<_>>();

        for branch_match in matches {
            configuration.add_layer(Arc::clone(&branch_match.block.layer));

            let consumed_path_segments =
                Self::consumed_location_segments(&branch_match.matched_keys);
            let remaining_path_keys = &request_path_keys[branch_match.consumed_input_len..];
            let remaining_path_segments =
                &request_path_segments[consumed_path_segments.min(request_path_segments.len())..];

            Self::apply_nested_layers(
                &branch_match.block,
                remaining_path_keys,
                remaining_path_segments,
                ctx,
                configuration,
            );
        }
    }

    #[inline]
    fn matched_hostname_segments(keys: &[HostLookupKey]) -> Vec<String> {
        let mut hostname_segments = keys
            .iter()
            .filter_map(|key| match key {
                HostLookupKey::HostDomainLevel(segment) => Some(segment.clone()),
                HostLookupKey::HostDomainLevelWildcard => Some("*".to_string()),
                _ => None,
            })
            .collect::<Vec<_>>();

        hostname_segments.reverse();
        hostname_segments
    }

    #[inline]
    fn matched_location_segments(keys: &[HostLookupKey]) -> Vec<String> {
        keys.iter()
            .filter_map(|key| match key {
                HostLookupKey::LocationSegment(segment) if !segment.is_empty() => {
                    Some(segment.clone())
                }
                _ => None,
            })
            .collect()
    }

    #[inline]
    fn matched_conditionals(keys: &[HostLookupKey]) -> Vec<ServerConfigurationMatcherExpr> {
        let mut conditionals = Vec::new();

        for key in keys {
            if let HostLookupKey::Conditional(conditional) = key {
                conditionals.extend(conditional.exprs.clone());
            }
        }

        conditionals
    }

    #[inline]
    fn consumed_location_segments(keys: &[HostLookupKey]) -> usize {
        keys.iter()
            .filter(
                |key| matches!(key, HostLookupKey::LocationSegment(segment) if !segment.is_empty()),
            )
            .count()
    }

    #[inline]
    fn has_hostname_keys(keys: &[HostLookupKey]) -> bool {
        keys.iter().any(|key| {
            matches!(
                key,
                HostLookupKey::HostDomainLevel(_)
                    | HostLookupKey::HostDomainLevelWildcard
                    | HostLookupKey::HostnameEnd
            )
        })
    }
}

impl Default for ThreeStageResolver {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}
