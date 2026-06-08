use opentelemetry::{
    trace::{Link, SpanKind, TraceContextExt, TraceId},
    KeyValue,
};
use opentelemetry_sdk::trace::{Sampler, SamplingDecision, SamplingResult, ShouldSample};

use crate::config::{
    AttributeBasedDefaultAction, AttributeMatcher, AttributeSamplingRule, TraceSamplingConfig,
    TraceSamplingMode,
};

/// An attribute-based sampler that makes sampling decisions based on span
/// attributes provided at span creation time (via `builder_attributes`).
#[derive(Debug, Clone)]
struct AttributeBasedSampler {
    rules: Vec<AttributeSamplingRule>,
    default_action: AttributeBasedDefaultAction,
}

impl ShouldSample for AttributeBasedSampler {
    fn should_sample(
        &self,
        parent_context: Option<&opentelemetry::Context>,
        _trace_id: TraceId,
        _name: &str,
        _span_kind: &SpanKind,
        attributes: &[KeyValue],
        _links: &[Link],
    ) -> SamplingResult {
        let decision = if self.rules.iter().any(|rule| match &rule.matcher {
            AttributeMatcher::Exact(expected) => attributes.iter().any(|kv| {
                if kv.key.as_str() != rule.attribute.as_str() {
                    return false;
                }
                match &kv.value {
                    opentelemetry::Value::String(s) => s.as_ref() == expected.as_str(),
                    _ => false,
                }
            }),
            AttributeMatcher::Prefix(prefix) => attributes.iter().any(|kv| {
                if kv.key.as_str() != rule.attribute.as_str() {
                    return false;
                }
                match &kv.value {
                    opentelemetry::Value::String(s) => s.as_ref().starts_with(prefix.as_str()),
                    _ => false,
                }
            }),
            AttributeMatcher::Exists => attributes
                .iter()
                .any(|kv| kv.key.as_str() == rule.attribute.as_str()),
        }) {
            SamplingDecision::RecordAndSample
        } else {
            match self.default_action {
                AttributeBasedDefaultAction::Sample => SamplingDecision::RecordAndSample,
                AttributeBasedDefaultAction::Drop => SamplingDecision::Drop,
            }
        };

        SamplingResult {
            decision,
            attributes: vec![],
            trace_state: parent_context
                .map(|cx| cx.span().span_context().trace_state().clone())
                .unwrap_or_default(),
        }
    }
}

/// Build an OpenTelemetry `ShouldSample` implementation from the sampling config.
#[cfg_attr(test, allow(dead_code))]
pub(crate) fn build_sampler(config: &TraceSamplingConfig) -> Box<dyn ShouldSample> {
    match &config.mode {
        TraceSamplingMode::AlwaysOn => Box::new(Sampler::AlwaysOn),
        TraceSamplingMode::AlwaysOff => Box::new(Sampler::AlwaysOff),
        TraceSamplingMode::ParentBasedAlwaysOn => {
            Box::new(Sampler::ParentBased(Box::new(Sampler::AlwaysOn)))
        }
        TraceSamplingMode::TraceIdRatioBased { ratio } => {
            Box::new(Sampler::TraceIdRatioBased(*ratio))
        }
        TraceSamplingMode::ParentBasedTraceIdRatio { ratio } => Box::new(Sampler::ParentBased(
            Box::new(Sampler::TraceIdRatioBased(*ratio)),
        )),
        TraceSamplingMode::AttributeBased {
            rules,
            default_action,
        } => Box::new(AttributeBasedSampler {
            rules: rules.clone(),
            default_action: *default_action,
        }),
    }
}
