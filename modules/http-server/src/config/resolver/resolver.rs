use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashMap},
    net::IpAddr,
    sync::Arc,
};

use ferron_core::config::{
    layer::LayeredConfiguration, ServerConfigurationBlock, ServerConfigurationMatcherExpr,
};
use ferron_http::HttpContext;

use super::super::prepare::{
    PreparedConfiguration, PreparedHostConfigurationBlock, PreparedHostConfigurationErrorConfig,
    PreparedHostConfigurationMatch, PreparedHostConfigurationMatcher,
};
use super::matcher::{evaluate_matcher_conditions, CompiledMatcherExpr};
use super::types::{ResolutionResult, ResolvedLocationPath};

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct ConditionalLookupKey {
    exprs: Vec<ServerConfigurationMatcherExpr>,
    negated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum HostLookupKey {
    IsLoopback,
    IPv4Octet(u8),
    IPv6Octet(u8),
    HostDomainLevel(String),
    HostDomainLevelWildcard,
    HostnameEnd,
    LocationSegment(String),
    Conditional(ConditionalLookupKey),
}

impl HostLookupKey {
    fn is_predicate(&self) -> bool {
        matches!(self, Self::HostDomainLevelWildcard | Self::Conditional(_))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct HostLookupMultiKey(Vec<HostLookupKey>);

#[allow(clippy::non_canonical_partial_ord_impl)]
impl PartialOrd for HostLookupMultiKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        for index in 0..self.0.len().max(other.0.len()) {
            match (self.0.get(index), other.0.get(index)) {
                (Some(left), Some(right)) => {
                    let cmp = left.cmp(right);
                    if cmp != Ordering::Equal {
                        return Some(cmp);
                    }
                }
                _ => return None,
            }
        }

        Some(Ordering::Equal)
    }
}

impl Ord for HostLookupMultiKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.partial_cmp(other).unwrap_or(Ordering::Equal)
    }
}

#[derive(Debug, Clone)]
struct ConditionalMatcher {
    compiled_exprs: Vec<CompiledMatcherExpr>,
    negated: bool,
}

impl ConditionalMatcher {
    fn compile(key: &ConditionalLookupKey) -> Option<Self> {
        let compiled_exprs: Result<Vec<_>, _> = key
            .exprs
            .iter()
            .cloned()
            .map(CompiledMatcherExpr::new)
            .collect();

        compiled_exprs.ok().map(|compiled_exprs| Self {
            compiled_exprs,
            negated: key.negated,
        })
    }

    fn matches(&self, ctx: &HttpContext) -> bool {
        let matched = evaluate_matcher_conditions(&self.compiled_exprs, ctx);
        if self.negated {
            !matched
        } else {
            matched
        }
    }
}

#[derive(Debug, Clone)]
enum PredicateMatcher {
    HostDomainWildcard,
    Conditional(ConditionalMatcher),
}

impl PredicateMatcher {
    fn from_key(key: &HostLookupKey) -> Option<Self> {
        match key {
            HostLookupKey::HostDomainLevelWildcard => Some(Self::HostDomainWildcard),
            HostLookupKey::Conditional(conditional) => {
                ConditionalMatcher::compile(conditional).map(Self::Conditional)
            }
            _ => None,
        }
    }

    fn consumed_input_len(
        &self,
        input: &[HostLookupKey],
        index: usize,
        ctx: &HttpContext,
    ) -> Option<usize> {
        match self {
            Self::HostDomainWildcard => {
                if !matches!(input.get(index), Some(HostLookupKey::HostDomainLevel(_))) {
                    return None;
                }

                let mut consumed = 0;
                while matches!(
                    input.get(index + consumed),
                    Some(HostLookupKey::HostDomainLevel(_))
                ) {
                    consumed += 1;
                }

                Some(consumed)
            }
            Self::Conditional(conditional) => conditional.matches(ctx).then_some(0),
        }
    }
}

#[derive(Debug)]
struct PredicateChild<T> {
    key: HostLookupKey,
    matcher: PredicateMatcher,
    node: HostLookupNode<T>,
}

#[derive(Debug)]
struct HostLookupNode<T> {
    value: Option<T>,
    children_fixed: BTreeMap<HostLookupMultiKey, HostLookupNode<T>>,
    children_predicate: Vec<PredicateChild<T>>,
}

impl<T> Default for HostLookupNode<T> {
    fn default() -> Self {
        Self {
            value: None,
            children_fixed: BTreeMap::new(),
            children_predicate: Vec::new(),
        }
    }
}

#[derive(Debug)]
struct HostLookupTree<T> {
    root: HostLookupNode<T>,
}

#[derive(Debug)]
struct HostLookupMatch<'a, T> {
    value: &'a T,
    matched_keys: Vec<HostLookupKey>,
    consumed_input_len: usize,
}

impl<T> HostLookupTree<T> {
    fn new() -> Self {
        Self {
            root: HostLookupNode::default(),
        }
    }

    fn insert_node(&mut self, key: Vec<HostLookupKey>) -> &mut Option<T> {
        let mut current_node = &mut self.root;
        let mut key_iter = key.into_iter();
        let mut key_option = key_iter.next();

        while let Some(key) = key_option.take() {
            if key.is_predicate() {
                let index = if let Some(index) = current_node
                    .children_predicate
                    .iter()
                    .position(|child| child.key == key)
                {
                    index
                } else {
                    let matcher = PredicateMatcher::from_key(&key)
                        .expect("predicate keys must be convertible into predicate matchers");
                    current_node.children_predicate.push(PredicateChild {
                        key: key.clone(),
                        matcher,
                        node: HostLookupNode::default(),
                    });
                    current_node.children_predicate.len() - 1
                };

                current_node = &mut current_node.children_predicate[index].node;
                key_option = key_iter.next();
                continue;
            }

            let mut multi_key = HostLookupMultiKey(vec![key]);
            match current_node.children_fixed.entry(multi_key) {
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    let entry_key = entry.key();
                    for index in 1..=entry_key.0.len() {
                        if index == entry_key.0.len() {
                            key_option = key_iter.next();
                            current_node = unsafe {
                                std::mem::transmute::<&mut HostLookupNode<T>, &mut HostLookupNode<T>>(
                                    entry.get_mut(),
                                )
                            };
                            break;
                        }

                        key_option = key_iter.next();
                        let should_split = match &key_option {
                            Some(next_key) => next_key != &entry_key.0[index],
                            None => true,
                        };

                        if should_split {
                            let (mut existing_key, existing_value) = entry.remove_entry();
                            let existing_right =
                                HostLookupMultiKey(existing_key.0.split_off(index));
                            let mut children_fixed = BTreeMap::new();
                            children_fixed.insert(existing_right, existing_value);

                            current_node = current_node
                                .children_fixed
                                .entry(existing_key)
                                .or_insert_with(|| HostLookupNode {
                                    value: None,
                                    children_fixed,
                                    children_predicate: Vec::new(),
                                });
                            break;
                        }
                    }
                }
                std::collections::btree_map::Entry::Vacant(entry) => {
                    multi_key = entry.into_key();

                    key_option = key_iter.next();
                    while let Some(next_key) = &key_option {
                        if next_key.is_predicate() {
                            break;
                        }

                        multi_key
                            .0
                            .push(key_option.take().expect("missing key during insertion"));
                        key_option = key_iter.next();
                    }

                    current_node = current_node.children_fixed.entry(multi_key).or_default();
                }
            }
        }

        &mut current_node.value
    }

    fn get<'a>(&'a self, key: &[HostLookupKey], ctx: &HttpContext) -> Vec<HostLookupMatch<'a, T>> {
        let mut matches = Vec::new();
        let mut matched_keys = Vec::new();
        Self::collect_matches(&self.root, key, 0, ctx, &mut matched_keys, &mut matches);
        matches
    }

    fn collect_matches<'a>(
        node: &'a HostLookupNode<T>,
        input: &[HostLookupKey],
        index: usize,
        ctx: &HttpContext,
        matched_keys: &mut Vec<HostLookupKey>,
        matches: &mut Vec<HostLookupMatch<'a, T>>,
    ) {
        if let Some(value) = node.value.as_ref() {
            matches.push(HostLookupMatch {
                value,
                matched_keys: matched_keys.clone(),
                consumed_input_len: index,
            });
        }

        for predicate_child in &node.children_predicate {
            if !matches!(
                predicate_child.matcher,
                PredicateMatcher::HostDomainWildcard
            ) {
                continue;
            }

            let Some(consumed) = predicate_child
                .matcher
                .consumed_input_len(input, index, ctx)
            else {
                continue;
            };

            matched_keys.push(predicate_child.key.clone());
            Self::collect_matches(
                &predicate_child.node,
                input,
                index + consumed,
                ctx,
                matched_keys,
                matches,
            );
            matched_keys.pop();
        }

        if let Some((child_key, child_node)) = Self::find_matching_fixed_child(node, input, index) {
            matched_keys.extend(child_key.0.iter().cloned());
            Self::collect_matches(
                child_node,
                input,
                index + child_key.0.len(),
                ctx,
                matched_keys,
                matches,
            );
            matched_keys.truncate(matched_keys.len() - child_key.0.len());
        }

        for predicate_child in &node.children_predicate {
            if !matches!(predicate_child.matcher, PredicateMatcher::Conditional(_)) {
                continue;
            }

            let Some(consumed) = predicate_child
                .matcher
                .consumed_input_len(input, index, ctx)
            else {
                continue;
            };

            matched_keys.push(predicate_child.key.clone());
            Self::collect_matches(
                &predicate_child.node,
                input,
                index + consumed,
                ctx,
                matched_keys,
                matches,
            );
            matched_keys.pop();
        }
    }

    fn find_matching_fixed_child<'a>(
        node: &'a HostLookupNode<T>,
        input: &[HostLookupKey],
        index: usize,
    ) -> Option<(&'a HostLookupMultiKey, &'a HostLookupNode<T>)> {
        node.children_fixed.iter().find(|(child_key, _)| {
            input
                .get(index..)
                .is_some_and(|remaining| remaining.starts_with(&child_key.0))
        })
    }
}

#[derive(Debug, Clone, Default)]
struct ErrorHandlerStatusLookup<T> {
    catchall_values: Vec<T>,
    status_code_values: HashMap<u16, Vec<T>>,
}

impl<T> ErrorHandlerStatusLookup<T> {
    fn new() -> Self {
        Self {
            catchall_values: Vec::new(),
            status_code_values: HashMap::new(),
        }
    }

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
    pub fn new() -> Self {
        Self {
            global: None,
            generic_hosts: HostLookupTree::new(),
            scoped_hosts: HostLookupTree::new(),
        }
    }

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

    pub fn from_prepared_with_global(
        prepared: PreparedConfiguration,
        global: Arc<ServerConfigurationBlock>,
    ) -> Self {
        let mut resolver = Self::from_prepared(prepared);
        resolver.global = Some(global);
        resolver
    }

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

    pub fn global(&self) -> Option<Arc<ServerConfigurationBlock>> {
        self.global.clone()
    }

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

    fn generic_host_lookup_key(hostname: Option<&str>) -> Vec<HostLookupKey> {
        let mut key = Vec::new();

        if let Some(hostname) = hostname {
            key.extend(Self::hostname_lookup_keys(hostname));
        }

        key
    }

    fn scoped_host_lookup_key(ip: IpAddr, hostname: Option<&str>) -> Vec<HostLookupKey> {
        let mut key = Self::ip_lookup_keys(ip);

        if let Some(hostname) = hostname {
            key.extend(Self::hostname_lookup_keys(hostname));
        }

        key
    }

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

    fn request_hostname_lookup_key(hostname: &str) -> Vec<HostLookupKey> {
        Self::hostname_lookup_keys(hostname)
    }

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

    fn path_lookup_key_from_segments(segments: &[String]) -> Vec<HostLookupKey> {
        let mut key = vec![HostLookupKey::LocationSegment(String::new())];
        key.extend(segments.iter().cloned().map(HostLookupKey::LocationSegment));
        key
    }

    fn split_path_segments(path: &str) -> Vec<String> {
        path.trim_start_matches('/')
            .split('/')
            .filter(|segment| !segment.is_empty())
            .map(str::to_string)
            .collect()
    }

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

    fn compile_error_block(
        error_config: &PreparedHostConfigurationErrorConfig,
    ) -> Arc<CompiledBlock> {
        Self::compile_block(Arc::new(error_config.config.clone()))
    }

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

    fn matched_conditionals(keys: &[HostLookupKey]) -> Vec<ServerConfigurationMatcherExpr> {
        let mut conditionals = Vec::new();

        for key in keys {
            if let HostLookupKey::Conditional(conditional) = key {
                conditionals.extend(conditional.exprs.clone());
            }
        }

        conditionals
    }

    fn consumed_location_segments(keys: &[HostLookupKey]) -> usize {
        keys.iter()
            .filter(
                |key| matches!(key, HostLookupKey::LocationSegment(segment) if !segment.is_empty()),
            )
            .count()
    }

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
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use ferron_core::config::{
        ServerConfigurationDirectiveEntry, ServerConfigurationMatcherOperand,
        ServerConfigurationMatcherOperator, ServerConfigurationValue,
    };
    use ferron_http::HttpRequest;
    use ferron_observability::CompositeEventSink;
    use http_body_util::{BodyExt, Empty};
    use rustc_hash::FxHashMap;
    use typemap_rev::TypeMap;

    use super::*;
    use crate::config::prepare::HostConfigs;

    fn make_test_context(req: HttpRequest, hostname: &str) -> HttpContext {
        HttpContext {
            req: Some(req),
            res: None,
            events: CompositeEventSink::new(Vec::new()),
            configuration: LayeredConfiguration::default(),
            hostname: Some(hostname.to_string()),
            variables: FxHashMap::default(),
            previous_error: None,
            original_uri: None,
            routing_uri: None,
            encrypted: false,
            local_address: "127.0.0.1:80".parse().unwrap(),
            remote_address: "127.0.0.1:12345".parse().unwrap(),
            auth_user: None,
            https_port: None,
            extensions: TypeMap::new(),
        }
    }

    fn empty_request() -> HttpRequest {
        http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync())
    }

    fn string_block(name: &str, value: &str) -> PreparedHostConfigurationBlock {
        let mut directives = HashMap::new();
        directives.insert(
            name.to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String(value.to_string(), None)],
                children: None,
                span: None,
            }],
        );

        PreparedHostConfigurationBlock {
            directives: Arc::new(directives),
            matches: Vec::new(),
            error_config: Vec::new(),
        }
    }

    fn read_string(config: &LayeredConfiguration, name: &str) -> Option<String> {
        config
            .get_value(name, true)
            .and_then(|value| value.as_str())
            .map(str::to_string)
    }

    #[test]
    fn layers_generic_ip_and_exact_host_blocks_by_specificity() {
        let mut generic_hosts = HostConfigs::new();
        generic_hosts.insert(None, Arc::new(string_block("generic_default", "yes")));
        generic_hosts.insert(
            Some("example.com".to_string()),
            Arc::new(string_block("generic_host", "yes")),
        );

        let mut scoped_hosts = HostConfigs::new();
        scoped_hosts.insert(None, Arc::new(string_block("ip_default", "yes")));
        scoped_hosts.insert(
            Some("example.com".to_string()),
            Arc::new(string_block("ip_host", "yes")),
        );

        let mut prepared = PreparedConfiguration::new();
        prepared.insert(None, generic_hosts);
        prepared.insert(Some("127.0.0.1".parse().unwrap()), scoped_hosts);

        let resolver = ThreeStageResolver::from_prepared(prepared);
        let result = resolver
            .resolve(
                "127.0.0.1".parse().unwrap(),
                "example.com",
                "/",
                &make_test_context(empty_request(), "example.com"),
            )
            .expect("request should resolve");

        assert_eq!(
            read_string(&result.configuration, "generic_default").as_deref(),
            Some("yes")
        );
        assert_eq!(
            read_string(&result.configuration, "ip_default").as_deref(),
            Some("yes")
        );
        assert_eq!(
            read_string(&result.configuration, "generic_host").as_deref(),
            Some("yes")
        );
        assert_eq!(
            read_string(&result.configuration, "ip_host").as_deref(),
            Some("yes")
        );
        assert_eq!(
            result.location_path.hostname_segments,
            vec!["example".to_string(), "com".to_string()]
        );
    }

    #[test]
    fn resolves_wildcard_hosts_using_lookup_tree_keys() {
        let mut hosts = HostConfigs::new();
        hosts.insert(
            Some("*.example.com".to_string()),
            Arc::new(string_block("host", "wildcard")),
        );
        hosts.insert(None, Arc::new(string_block("host", "default")));

        let mut prepared = PreparedConfiguration::new();
        prepared.insert(Some("127.0.0.1".parse().unwrap()), hosts);

        let resolver = ThreeStageResolver::from_prepared(prepared);
        let wildcard = resolver
            .resolve(
                "127.0.0.1".parse().unwrap(),
                "deep.api.example.com",
                "/",
                &make_test_context(empty_request(), "deep.api.example.com"),
            )
            .expect("wildcard host should resolve");

        assert_eq!(
            read_string(&wildcard.configuration, "host").as_deref(),
            Some("wildcard")
        );
        assert_eq!(
            wildcard.location_path.hostname_segments,
            vec!["*".to_string(), "example".to_string(), "com".to_string()]
        );
    }

    #[test]
    fn layers_multiple_matching_locations_additively() {
        let host = PreparedHostConfigurationBlock {
            directives: Arc::new(HashMap::new()),
            matches: vec![
                PreparedHostConfigurationMatch {
                    matcher: PreparedHostConfigurationMatcher::Location("/".to_string()),
                    config: Arc::new(string_block("root_location", "yes")),
                },
                PreparedHostConfigurationMatch {
                    matcher: PreparedHostConfigurationMatcher::Location("/api".to_string()),
                    config: Arc::new(string_block("api_location", "yes")),
                },
            ],
            error_config: Vec::new(),
        };

        let mut hosts = HostConfigs::new();
        hosts.insert(Some("example.com".to_string()), Arc::new(host));

        let mut prepared = PreparedConfiguration::new();
        prepared.insert(Some("127.0.0.1".parse().unwrap()), hosts);

        let resolver = ThreeStageResolver::from_prepared(prepared);
        let result = resolver
            .resolve(
                "127.0.0.1".parse().unwrap(),
                "example.com",
                "/api/users",
                &make_test_context(empty_request(), "example.com"),
            )
            .expect("request should resolve");

        assert_eq!(
            read_string(&result.configuration, "root_location").as_deref(),
            Some("yes")
        );
        assert_eq!(
            read_string(&result.configuration, "api_location").as_deref(),
            Some("yes")
        );
        assert_eq!(result.location_path.path_segments, vec!["api".to_string()]);
    }

    #[test]
    fn layers_multiple_matching_conditionals_additively() {
        let expr_get = ServerConfigurationMatcherExpr {
            left: ServerConfigurationMatcherOperand::Identifier("request.method".to_string()),
            right: ServerConfigurationMatcherOperand::String("GET".to_string()),
            op: ServerConfigurationMatcherOperator::Eq,
        };
        let expr_root = ServerConfigurationMatcherExpr {
            left: ServerConfigurationMatcherOperand::Identifier("request.uri.path".to_string()),
            right: ServerConfigurationMatcherOperand::String("/".to_string()),
            op: ServerConfigurationMatcherOperator::Eq,
        };

        let block = PreparedHostConfigurationBlock {
            directives: Arc::new(HashMap::new()),
            matches: vec![
                PreparedHostConfigurationMatch {
                    matcher: PreparedHostConfigurationMatcher::IfConditional(vec![expr_get]),
                    config: Arc::new(string_block("if_get", "yes")),
                },
                PreparedHostConfigurationMatch {
                    matcher: PreparedHostConfigurationMatcher::IfConditional(vec![expr_root]),
                    config: Arc::new(string_block("if_root", "yes")),
                },
            ],
            error_config: Vec::new(),
        };

        let mut hosts = HostConfigs::new();
        hosts.insert(Some("example.com".to_string()), Arc::new(block));

        let mut prepared = PreparedConfiguration::new();
        prepared.insert(Some("127.0.0.1".parse().unwrap()), hosts);

        let resolver = ThreeStageResolver::from_prepared(prepared);
        let result = resolver
            .resolve(
                "127.0.0.1".parse().unwrap(),
                "example.com",
                "/",
                &make_test_context(empty_request(), "example.com"),
            )
            .expect("request should resolve");

        assert_eq!(
            read_string(&result.configuration, "if_get").as_deref(),
            Some("yes")
        );
        assert_eq!(
            read_string(&result.configuration, "if_root").as_deref(),
            Some("yes")
        );
        assert_eq!(result.location_path.conditionals.len(), 2);
    }

    #[test]
    fn resolves_nested_location_inside_a_conditional_scope() {
        let expr_post = ServerConfigurationMatcherExpr {
            left: ServerConfigurationMatcherOperand::Identifier("request.method".to_string()),
            right: ServerConfigurationMatcherOperand::String("POST".to_string()),
            op: ServerConfigurationMatcherOperator::Eq,
        };

        let conditional_block = PreparedHostConfigurationBlock {
            directives: Arc::new(HashMap::new()),
            matches: vec![PreparedHostConfigurationMatch {
                matcher: PreparedHostConfigurationMatcher::Location("/admin".to_string()),
                config: Arc::new(string_block("nested", "hit")),
            }],
            error_config: Vec::new(),
        };

        let host = PreparedHostConfigurationBlock {
            directives: Arc::new(HashMap::new()),
            matches: vec![PreparedHostConfigurationMatch {
                matcher: PreparedHostConfigurationMatcher::IfConditional(vec![expr_post]),
                config: Arc::new(conditional_block),
            }],
            error_config: Vec::new(),
        };

        let mut hosts = HostConfigs::new();
        hosts.insert(Some("example.com".to_string()), Arc::new(host));

        let mut prepared = PreparedConfiguration::new();
        prepared.insert(Some("127.0.0.1".parse().unwrap()), hosts);

        let resolver = ThreeStageResolver::from_prepared(prepared);
        let mut request = http::Request::new(Empty::new().map_err(|e| match e {}).boxed_unsync());
        *request.method_mut() = http::Method::POST;

        let result = resolver
            .resolve(
                "127.0.0.1".parse().unwrap(),
                "example.com",
                "/admin/panel",
                &make_test_context(request, "example.com"),
            )
            .expect("request should resolve");

        assert_eq!(
            read_string(&result.configuration, "nested").as_deref(),
            Some("hit")
        );
        assert_eq!(
            result.location_path.path_segments,
            vec!["admin".to_string()]
        );
        assert_eq!(result.location_path.conditionals.len(), 1);
    }

    #[test]
    fn layers_error_handlers_from_matching_scopes() {
        let api_location = PreparedHostConfigurationBlock {
            directives: Arc::new(HashMap::new()),
            matches: Vec::new(),
            error_config: vec![PreparedHostConfigurationErrorConfig {
                error_code: Some(404),
                config: string_block("api_error", "yes"),
            }],
        };

        let host = PreparedHostConfigurationBlock {
            directives: Arc::new(HashMap::new()),
            matches: vec![PreparedHostConfigurationMatch {
                matcher: PreparedHostConfigurationMatcher::Location("/api".to_string()),
                config: Arc::new(api_location),
            }],
            error_config: vec![PreparedHostConfigurationErrorConfig {
                error_code: None,
                config: string_block("host_error", "yes"),
            }],
        };

        let mut hosts = HostConfigs::new();
        hosts.insert(Some("example.com".to_string()), Arc::new(host));

        let mut prepared = PreparedConfiguration::new();
        prepared.insert(Some("127.0.0.1".parse().unwrap()), hosts);

        let resolver = ThreeStageResolver::from_prepared(prepared);
        let result = resolver
            .resolve_error_scoped(
                "127.0.0.1".parse().unwrap(),
                "example.com",
                "/api/users",
                404,
                &make_test_context(empty_request(), "example.com"),
            )
            .expect("error resolution should succeed");

        assert_eq!(
            read_string(&result.configuration, "host_error").as_deref(),
            Some("yes")
        );
        assert_eq!(
            read_string(&result.configuration, "api_error").as_deref(),
            Some("yes")
        );
        assert_eq!(result.location_path.error_key, Some(404));
    }
}
