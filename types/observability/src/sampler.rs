use ferron_core::config::validator::ConfigurationValidatorContext;
use ferron_core::config::ServerConfigurationBlock;
use ferron_core::config::ServerConfigurationDirectiveEntry;
use ferron_core::config::ServerConfigurationValue;

use crate::{Parent, TraceAttributeValue};

/// Sampling mode for traces.
#[derive(Debug, Clone)]
pub enum TraceSamplingMode {
    /// Sample every trace.
    AlwaysOn,
    /// Sample no traces.
    AlwaysOff,
    /// Respect the parent span's sampling decision; AlwaysOn for root spans.
    ParentBasedAlwaysOn,
    /// Sample a fixed ratio of traces based on trace ID.
    TraceIdRatioBased { ratio: f64 },
    /// Parent-based with TraceIdRatioBased for root spans.
    ParentBasedTraceIdRatio { ratio: f64 },
    /// Sample based on span attributes set before the span is built.
    AttributeBased {
        rules: Vec<AttributeSamplingRule>,
        default_action: AttributeBasedDefaultAction,
    },
}

/// A rule for attribute-based sampling.
#[derive(Debug, Clone)]
pub struct AttributeSamplingRule {
    /// The attribute key to match against.
    pub attribute: String,
    /// How to match the attribute value.
    pub matcher: AttributeMatcher,
}

/// Matcher for attribute-based sampling rules.
#[derive(Debug, Clone)]
pub enum AttributeMatcher {
    /// Exact string match.
    Exact(String),
    /// Prefix match.
    Prefix(String),
    /// Match if the attribute exists (any value).
    Exists,
}

/// Default action when no attribute-based sampling rules match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AttributeBasedDefaultAction {
    /// Sample spans that don't match any rule.
    Sample,
    /// Drop spans that don't match any rule.
    #[default]
    Drop,
}

/// Trace sampling configuration.
#[derive(Debug, Clone)]
pub struct TraceSamplingConfig {
    pub mode: TraceSamplingMode,
}

impl Default for TraceSamplingConfig {
    #[inline]
    fn default() -> Self {
        Self {
            mode: TraceSamplingMode::ParentBasedAlwaysOn,
        }
    }
}

/// Trace sampler that evaluates sampling decisions without OTel SDK dependencies.
#[derive(Debug, Clone)]
pub struct TraceSampler {
    mode: TraceSamplingMode,
}

impl TraceSampler {
    /// Create a new sampler from a configuration.
    #[inline]
    pub fn new(config: &TraceSamplingConfig) -> Self {
        Self {
            mode: config.mode.clone(),
        }
    }

    /// Evaluate whether a span should be sampled.
    ///
    /// - `parent`: The parent span reference, if any.
    /// - `trace_id`: The 32-byte trace ID from the event trace context.
    /// - `builder_attributes`: Attributes set on the SpanBuilder before build.
    #[inline]
    pub fn should_sample(
        &self,
        parent: Option<&Parent>,
        trace_id: Option<&[u8; 32]>,
        builder_attributes: &[(&str, &TraceAttributeValue)],
    ) -> bool {
        match &self.mode {
            TraceSamplingMode::AlwaysOn => true,
            TraceSamplingMode::AlwaysOff => false,
            TraceSamplingMode::ParentBasedAlwaysOn => self.parent_based_always_on(parent),
            TraceSamplingMode::TraceIdRatioBased { ratio } => {
                trace_id.is_none_or(|tid| trace_id_ratio_sample(tid, *ratio))
            }
            TraceSamplingMode::ParentBasedTraceIdRatio { ratio } => {
                self.parent_based_trace_id_ratio(parent, trace_id, *ratio)
            }
            TraceSamplingMode::AttributeBased {
                rules,
                default_action,
            } => attribute_based_sample(rules, *default_action, builder_attributes),
        }
    }

    #[inline]
    fn parent_based_always_on(&self, parent: Option<&Parent>) -> bool {
        match parent {
            Some(Parent::ById { sampled, .. }) => sampled.unwrap_or(true),
            _ => true,
        }
    }

    #[inline]
    fn parent_based_trace_id_ratio(
        &self,
        parent: Option<&Parent>,
        trace_id: Option<&[u8; 32]>,
        ratio: f64,
    ) -> bool {
        match parent {
            Some(Parent::ById { sampled, .. }) => sampled.unwrap_or(true),
            None => trace_id.is_none_or(|tid| trace_id_ratio_sample(tid, ratio)),
            Some(Parent::ByKey(_)) => true,
        }
    }
}

/// Deterministic ratio-based sampling using a simple hash of the trace ID.
///
/// Maps the 32-byte trace ID to a float in [0.0, 1.0) using a fast,
/// deterministic algorithm. Returns `true` if the hash is less than the ratio.
#[inline]
fn trace_id_ratio_sample(trace_id: &[u8; 32], ratio: f64) -> bool {
    if ratio <= 0.0 {
        return false;
    }
    if ratio >= 1.0 {
        return true;
    }

    // xxh3 is deterministic across ALL architectures and has perfect avalanche
    let hash = xxhash_rust::xxh3::xxh3_64(trace_id);

    // Normalize correctly to [0.0, 1.0)
    let normalized = (hash >> 11) as f64 / (1u64 << 53) as f64;

    normalized < ratio
}

/// Evaluate attribute-based sampling rules.
#[inline]
fn attribute_based_sample(
    rules: &[AttributeSamplingRule],
    default_action: AttributeBasedDefaultAction,
    builder_attributes: &[(&str, &TraceAttributeValue)],
) -> bool {
    if rules.is_empty() {
        return matches!(default_action, AttributeBasedDefaultAction::Sample);
    }

    let matched = rules.iter().any(|rule| {
        builder_attributes.iter().any(|(key, value)| {
            if *key != rule.attribute.as_str() {
                return false;
            }
            match &rule.matcher {
                AttributeMatcher::Exact(expected) => match value {
                    TraceAttributeValue::String(s) => s == expected,
                    TraceAttributeValue::StaticStr(s) => *s == expected.as_str(),
                    _ => false,
                },
                AttributeMatcher::Prefix(prefix) => match value {
                    TraceAttributeValue::String(s) => s.starts_with(prefix.as_str()),
                    TraceAttributeValue::StaticStr(s) => s.starts_with(prefix.as_str()),
                    _ => false,
                },
                AttributeMatcher::Exists => true,
            }
        })
    });

    if matched {
        true
    } else {
        matches!(default_action, AttributeBasedDefaultAction::Sample)
    }
}

/// Parse the `trace_sampling` directive from an HTTP config block.
///
/// Expected format:
/// ```text
/// trace_sampling "parentbased_traceidratio" {
///     ratio 0.1
/// }
/// ```
pub fn parse_trace_sampling_config(
    entry: &ServerConfigurationDirectiveEntry,
) -> TraceSamplingConfig {
    let mode = match entry.args.first().and_then(|v| v.as_str()) {
        Some("always_on") => TraceSamplingMode::AlwaysOn,
        Some("always_off") => TraceSamplingMode::AlwaysOff,
        Some("parentbased_always_on") => TraceSamplingMode::ParentBasedAlwaysOn,
        Some("traceidratio") => {
            let ratio = entry
                .children
                .as_ref()
                .and_then(|c| c.get_value("ratio"))
                .and_then(|v| v.as_float())
                .unwrap_or(1.0);
            TraceSamplingMode::TraceIdRatioBased { ratio }
        }
        Some("parentbased_traceidratio") => {
            let ratio = entry
                .children
                .as_ref()
                .and_then(|c| c.get_value("ratio"))
                .and_then(|v| v.as_float())
                .unwrap_or(1.0);
            TraceSamplingMode::ParentBasedTraceIdRatio { ratio }
        }
        Some("attribute_based") => {
            let default_action = entry
                .children
                .as_ref()
                .and_then(|c| c.get_value("default_action"))
                .and_then(|v| v.as_str())
                .and_then(parse_attribute_based_default_action)
                .unwrap_or_default();
            let rules = entry
                .children
                .as_ref()
                .map(parse_attribute_sampling_rules)
                .unwrap_or_default();
            TraceSamplingMode::AttributeBased {
                rules,
                default_action,
            }
        }
        _ => return TraceSamplingConfig::default(),
    };

    TraceSamplingConfig { mode }
}

/// Parse attribute sampling rules from a `rules { ... }` block.
fn parse_attribute_sampling_rules(
    children: &ServerConfigurationBlock,
) -> Vec<AttributeSamplingRule> {
    let Some(rules_entries) = children.directives.get("rules") else {
        return Vec::new();
    };
    let Some(rules_block) = rules_entries.first().and_then(|e| e.children.as_ref()) else {
        return Vec::new();
    };

    let Some(rule_entries) = rules_block.directives.get("rule") else {
        return Vec::new();
    };

    let mut rules = Vec::new();
    for entry in rule_entries {
        let Some(match_type) = entry.args.first().and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(attribute) = entry.args.get(1).and_then(|v| v.as_str()) else {
            continue;
        };

        let matcher = match match_type {
            "exact" => {
                let Some(value) = entry.args.get(2).and_then(|v| v.as_str()) else {
                    continue;
                };
                AttributeMatcher::Exact(value.to_string())
            }
            "prefix" => {
                let Some(value) = entry.args.get(2).and_then(|v| v.as_str()) else {
                    continue;
                };
                AttributeMatcher::Prefix(value.to_string())
            }
            "exists" => AttributeMatcher::Exists,
            _ => continue,
        };

        rules.push(AttributeSamplingRule {
            attribute: attribute.to_string(),
            matcher,
        });
    }

    rules
}

/// Parse a `default_action` directive value for attribute-based sampling.
fn parse_attribute_based_default_action(value: &str) -> Option<AttributeBasedDefaultAction> {
    match value {
        "sample" => Some(AttributeBasedDefaultAction::Sample),
        "drop" => Some(AttributeBasedDefaultAction::Drop),
        _ => None,
    }
}

/// Validate a `trace_sampling` directive.
pub fn validate_trace_sampling_directive(
    entry: &ServerConfigurationDirectiveEntry,
    validator_ctx: &mut ConfigurationValidatorContext,
) -> Result<(), Box<dyn std::error::Error>> {
    let mode = entry.args.first().and_then(|v| v.as_str());
    match mode {
        Some("always_on" | "always_off" | "parentbased_always_on") => {}
        Some("traceidratio" | "parentbased_traceidratio") => {
            if let Some(children) = &entry.children {
                if let Some(ratio_entries) = children.directives.get("ratio") {
                    for ratio_entry in ratio_entries {
                        if ratio_entry.args.len() != 1 {
                            let err: Box<dyn std::error::Error> =
                                "Invalid `ratio` directive: expected exactly 1 argument (a float between 0.0 and 1.0)"
                                    .to_string()
                                    .into();
                            Err(err)?;
                        } else if !matches!(
                            &ratio_entry.args[0],
                            ServerConfigurationValue::Float(_, _)
                        ) {
                            let err: Box<dyn std::error::Error> =
                                "Invalid `ratio` value: must be a float between 0.0 and 1.0"
                                    .to_string()
                                    .into();
                            Err(err)?;
                        } else if let ServerConfigurationValue::Float(r, _) = &ratio_entry.args[0] {
                            if *r < 0.0 || *r > 1.0 {
                                let err: Box<dyn std::error::Error> =
                                    "Invalid `ratio` value: must be between 0.0 and 1.0"
                                        .to_string()
                                        .into();
                                Err(err)?;
                            }
                        }
                    }
                }
            }
        }
        Some("attribute_based") => {
            if let Some(children) = &entry.children {
                if let Some(da_entries) = children.directives.get("default_action") {
                    for da_entry in da_entries {
                        if da_entry.args.len() != 1 {
                            let err: Box<dyn std::error::Error> =
                                "Invalid `default_action` directive: expected exactly 1 argument ('sample' or 'drop')"
                                    .to_string()
                                    .into();
                            Err(err)?;
                        } else if let Some(value) = da_entry.args[0].as_str() {
                            if value != "sample" && value != "drop" {
                                let err: Box<dyn std::error::Error> = format!(
                                    "Invalid `default_action` value '{}': must be 'sample' or 'drop'",
                                    value
                                )
                                .into();
                                Err(err)?;
                            }
                        } else {
                            let err: Box<dyn std::error::Error> =
                                "Invalid `default_action` value: must be a string ('sample' or 'drop')"
                                    .to_string()
                                    .into();
                            Err(err)?;
                        }
                    }
                }

                if children.directives.get("default_action").is_none() {
                    validator_ctx.add_best_practice_violation(
                        "`attribute_based` sampling without explicit `default_action`; \
                         spans not matching any rule are silently dropped",
                        entry.span.clone(),
                    );
                }
            }

            if let Some(children) = &entry.children {
                if let Some(rules_entries) = children.directives.get("rules") {
                    for rules_entry in rules_entries {
                        if let Some(rules_block) = &rules_entry.children {
                            if let Some(rule_entries) = rules_block.directives.get("rule") {
                                for rule_entry in rule_entries {
                                    if rule_entry.args.len() < 2 || rule_entry.args.len() > 3 {
                                        let err: Box<dyn std::error::Error> = format!(
                                            "Invalid `rule` directive: expected 2 or 3 arguments (match_type, attribute, [value]), got {}",
                                            rule_entry.args.len()
                                        )
                                        .into();
                                        Err(err)?;
                                    }
                                    if let Some(match_type) =
                                        rule_entry.args.first().and_then(|v| v.as_str())
                                    {
                                        match match_type {
                                            "exact" | "prefix" => {
                                                if rule_entry.args.len() != 3 {
                                                    let err: Box<dyn std::error::Error> = format!(
                                                        "Invalid `{}` rule: expected 3 arguments (match_type, attribute, value), got {}",
                                                        match_type,
                                                        rule_entry.args.len()
                                                    )
                                                    .into();
                                                    Err(err)?;
                                                }
                                            }
                                            "exists" => {
                                                if rule_entry.args.len() != 2 {
                                                    let err: Box<dyn std::error::Error> = format!(
                                                        "Invalid `exists` rule: expected 2 arguments (match_type, attribute), got {}",
                                                        rule_entry.args.len()
                                                    )
                                                    .into();
                                                    Err(err)?;
                                                }
                                            }
                                            other => {
                                                let err: Box<dyn std::error::Error> = format!(
                                                    "Invalid rule match type '{}': must be 'exact', 'prefix', or 'exists'",
                                                    other
                                                )
                                                .into();
                                                Err(err)?;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        Some(other) => {
            let err: Box<dyn std::error::Error> = format!(
                "Invalid trace sampling mode '{}': must be one of 'always_on', 'always_off', 'parentbased_always_on', 'traceidratio', 'parentbased_traceidratio', 'attribute_based'",
                other
            )
            .into();
            Err(err)?;
        }
        None => {
            let err: Box<dyn std::error::Error> =
                "The `trace_sampling` directive requires a mode argument"
                    .to_string()
                    .into();
            Err(err)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attrs<'a>(
        pairs: &'a [(&str, TraceAttributeValue)],
    ) -> Vec<(&'a str, &'a TraceAttributeValue)> {
        pairs.iter().map(|(k, v)| (*k, v)).collect()
    }

    #[test]
    fn always_on_samples_everything() {
        let sampler = TraceSampler::new(&TraceSamplingConfig {
            mode: TraceSamplingMode::AlwaysOn,
        });
        assert!(sampler.should_sample(None, None, &[]));
    }

    #[test]
    fn always_off_samples_nothing() {
        let sampler = TraceSampler::new(&TraceSamplingConfig {
            mode: TraceSamplingMode::AlwaysOff,
        });
        assert!(!sampler.should_sample(None, None, &[]));
    }

    #[test]
    fn parentbased_always_on_root_span() {
        let sampler = TraceSampler::new(&TraceSamplingConfig {
            mode: TraceSamplingMode::ParentBasedAlwaysOn,
        });
        assert!(sampler.should_sample(None, None, &[]));
    }

    #[test]
    fn parentbased_always_on_parent_sampled() {
        let sampler = TraceSampler::new(&TraceSamplingConfig {
            mode: TraceSamplingMode::ParentBasedAlwaysOn,
        });
        let parent = Parent::ById {
            trace_id: "abc".to_string(),
            span_id: "def".to_string(),
            sampled: Some(true),
            baggage: None,
        };
        assert!(sampler.should_sample(Some(&parent), None, &[]));
    }

    #[test]
    fn parentbased_always_on_parent_not_sampled() {
        let sampler = TraceSampler::new(&TraceSamplingConfig {
            mode: TraceSamplingMode::ParentBasedAlwaysOn,
        });
        let parent = Parent::ById {
            trace_id: "abc".to_string(),
            span_id: "def".to_string(),
            sampled: Some(false),
            baggage: None,
        };
        assert!(!sampler.should_sample(Some(&parent), None, &[]));
    }

    #[test]
    fn parentbased_always_on_parent_sampled_none() {
        let sampler = TraceSampler::new(&TraceSamplingConfig {
            mode: TraceSamplingMode::ParentBasedAlwaysOn,
        });
        let parent = Parent::ById {
            trace_id: "abc".to_string(),
            span_id: "def".to_string(),
            sampled: None,
            baggage: None,
        };
        assert!(sampler.should_sample(Some(&parent), None, &[]));
    }

    #[test]
    fn traceidratio_zero_always_off() {
        let sampler = TraceSampler::new(&TraceSamplingConfig {
            mode: TraceSamplingMode::TraceIdRatioBased { ratio: 0.0 },
        });
        let trace_id = [1u8; 32];
        assert!(!sampler.should_sample(None, Some(&trace_id), &[]));
    }

    #[test]
    fn traceidratio_one_always_on() {
        let sampler = TraceSampler::new(&TraceSamplingConfig {
            mode: TraceSamplingMode::TraceIdRatioBased { ratio: 1.0 },
        });
        let trace_id = [1u8; 32];
        assert!(sampler.should_sample(None, Some(&trace_id), &[]));
    }

    #[test]
    fn parentbased_traceidratio_parent_sampled() {
        let sampler = TraceSampler::new(&TraceSamplingConfig {
            mode: TraceSamplingMode::ParentBasedTraceIdRatio { ratio: 0.0 },
        });
        let parent = Parent::ById {
            trace_id: "abc".to_string(),
            span_id: "def".to_string(),
            sampled: Some(true),
            baggage: None,
        };
        assert!(sampler.should_sample(Some(&parent), None, &[]));
    }

    #[test]
    fn parentbased_traceidratio_parent_not_sampled() {
        let sampler = TraceSampler::new(&TraceSamplingConfig {
            mode: TraceSamplingMode::ParentBasedTraceIdRatio { ratio: 1.0 },
        });
        let parent = Parent::ById {
            trace_id: "abc".to_string(),
            span_id: "def".to_string(),
            sampled: Some(false),
            baggage: None,
        };
        assert!(!sampler.should_sample(Some(&parent), None, &[]));
    }

    #[test]
    fn parentbased_traceidratio_root_uses_ratio() {
        let sampler_zero = TraceSampler::new(&TraceSamplingConfig {
            mode: TraceSamplingMode::ParentBasedTraceIdRatio { ratio: 0.0 },
        });
        let sampler_one = TraceSampler::new(&TraceSamplingConfig {
            mode: TraceSamplingMode::ParentBasedTraceIdRatio { ratio: 1.0 },
        });
        let trace_id = [42u8; 32];
        assert!(!sampler_zero.should_sample(None, Some(&trace_id), &[]));
        assert!(sampler_one.should_sample(None, Some(&trace_id), &[]));
    }

    #[test]
    fn attribute_based_exact_match() {
        let sampler = TraceSampler::new(&TraceSamplingConfig {
            mode: TraceSamplingMode::AttributeBased {
                rules: vec![AttributeSamplingRule {
                    attribute: "http.request.method".to_string(),
                    matcher: AttributeMatcher::Exact("POST".to_string()),
                }],
                default_action: AttributeBasedDefaultAction::Drop,
            },
        });
        let post = vec![(
            "http.request.method",
            TraceAttributeValue::String("POST".to_string()),
        )];
        assert!(sampler.should_sample(None, None, &attrs(&post)));

        let get = vec![(
            "http.request.method",
            TraceAttributeValue::String("GET".to_string()),
        )];
        assert!(!sampler.should_sample(None, None, &attrs(&get)));
    }

    #[test]
    fn attribute_based_prefix_match() {
        let sampler = TraceSampler::new(&TraceSamplingConfig {
            mode: TraceSamplingMode::AttributeBased {
                rules: vec![AttributeSamplingRule {
                    attribute: "url.path".to_string(),
                    matcher: AttributeMatcher::Prefix("/api/".to_string()),
                }],
                default_action: AttributeBasedDefaultAction::Drop,
            },
        });
        let api = vec![(
            "url.path",
            TraceAttributeValue::String("/api/users".to_string()),
        )];
        assert!(sampler.should_sample(None, None, &attrs(&api)));

        let static_file = vec![(
            "url.path",
            TraceAttributeValue::String("/static/file".to_string()),
        )];
        assert!(!sampler.should_sample(None, None, &attrs(&static_file)));
    }

    #[test]
    fn attribute_based_exists_match() {
        let sampler = TraceSampler::new(&TraceSamplingConfig {
            mode: TraceSamplingMode::AttributeBased {
                rules: vec![AttributeSamplingRule {
                    attribute: "error.type".to_string(),
                    matcher: AttributeMatcher::Exists,
                }],
                default_action: AttributeBasedDefaultAction::Drop,
            },
        });
        let with_error = vec![("error.type", TraceAttributeValue::String("500".to_string()))];
        assert!(sampler.should_sample(None, None, &attrs(&with_error)));

        let without_error = vec![(
            "status.code",
            TraceAttributeValue::String("200".to_string()),
        )];
        assert!(!sampler.should_sample(None, None, &attrs(&without_error)));
    }

    #[test]
    fn attribute_based_default_action_sample() {
        let sampler = TraceSampler::new(&TraceSamplingConfig {
            mode: TraceSamplingMode::AttributeBased {
                rules: vec![AttributeSamplingRule {
                    attribute: "http.request.method".to_string(),
                    matcher: AttributeMatcher::Exact("POST".to_string()),
                }],
                default_action: AttributeBasedDefaultAction::Sample,
            },
        });
        let get = vec![(
            "http.request.method",
            TraceAttributeValue::String("GET".to_string()),
        )];
        assert!(sampler.should_sample(None, None, &attrs(&get)));
    }

    #[test]
    fn attribute_based_empty_rules_with_default() {
        let sampler_sample = TraceSampler::new(&TraceSamplingConfig {
            mode: TraceSamplingMode::AttributeBased {
                rules: vec![],
                default_action: AttributeBasedDefaultAction::Sample,
            },
        });
        assert!(sampler_sample.should_sample(None, None, &[]));

        let sampler_drop = TraceSampler::new(&TraceSamplingConfig {
            mode: TraceSamplingMode::AttributeBased {
                rules: vec![],
                default_action: AttributeBasedDefaultAction::Drop,
            },
        });
        assert!(!sampler_drop.should_sample(None, None, &[]));
    }

    #[test]
    fn trace_id_ratio_deterministic() {
        let sampler = TraceSampler::new(&TraceSamplingConfig {
            mode: TraceSamplingMode::TraceIdRatioBased { ratio: 0.5 },
        });
        let trace_id = [123u8; 32];
        let first = sampler.should_sample(None, Some(&trace_id), &[]);
        let second = sampler.should_sample(None, Some(&trace_id), &[]);
        assert_eq!(first, second);
    }
}
