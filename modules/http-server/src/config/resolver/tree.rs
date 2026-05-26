use std::{cmp::Ordering, collections::BTreeMap};

use ferron_core::config::ServerConfigurationMatcherExpr;
use ferron_http::HttpContext;

use super::matcher::{evaluate_matcher_conditions, CompiledMatcherExpr};

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct ConditionalLookupKey {
    pub exprs: Vec<ServerConfigurationMatcherExpr>,
    pub negated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum HostLookupKey {
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
    #[inline]
    fn is_predicate(&self) -> bool {
        matches!(self, Self::HostDomainLevelWildcard | Self::Conditional(_))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostLookupMultiKey(Vec<HostLookupKey>);

#[allow(clippy::non_canonical_partial_ord_impl)]
impl PartialOrd for HostLookupMultiKey {
    #[inline]
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
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        self.partial_cmp(other).unwrap_or(Ordering::Equal)
    }
}

#[derive(Debug, Clone)]
pub struct ConditionalMatcher {
    compiled_exprs: Vec<CompiledMatcherExpr>,
    negated: bool,
}

impl ConditionalMatcher {
    #[inline]
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

    #[inline]
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
pub enum PredicateMatcher {
    HostDomainWildcard,
    Conditional(ConditionalMatcher),
}

impl PredicateMatcher {
    #[inline]
    pub fn from_key(key: &HostLookupKey) -> Option<Self> {
        match key {
            HostLookupKey::HostDomainLevelWildcard => Some(Self::HostDomainWildcard),
            HostLookupKey::Conditional(conditional) => {
                ConditionalMatcher::compile(conditional).map(Self::Conditional)
            }
            _ => None,
        }
    }

    #[inline]
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
    #[inline]
    fn default() -> Self {
        Self {
            value: None,
            children_fixed: BTreeMap::new(),
            children_predicate: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub struct HostLookupTree<T> {
    root: HostLookupNode<T>,
}

#[derive(Debug)]
pub struct HostLookupMatch<'a, T> {
    pub value: &'a T,
    pub matched_keys: Vec<HostLookupKey>,
    pub consumed_input_len: usize,
}

impl<T> HostLookupTree<T> {
    #[inline]
    pub fn new() -> Self {
        Self {
            root: HostLookupNode::default(),
        }
    }

    #[inline]
    pub fn insert_node(&mut self, key: Vec<HostLookupKey>) -> &mut Option<T> {
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

    #[inline]
    pub fn get<'a>(
        &'a self,
        key: &[HostLookupKey],
        ctx: &HttpContext,
    ) -> Vec<HostLookupMatch<'a, T>> {
        let mut matches = Vec::new();
        let mut matched_keys = Vec::new();
        Self::collect_matches(&self.root, key, 0, ctx, &mut matched_keys, &mut matches);
        matches
    }

    #[inline]
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

    #[inline]
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
