//! Per-host W3C Trace Context and trace sampling settings.
//!
//! These come from the `http` block (`trace { generate, trust_request }` and
//! `trace_sampling`) and must be known before the pipeline runs: the request
//! span and the trace context propagated to upstreams are created at the very
//! start of request handling, before location/conditional resolution. They are
//! therefore resolved from host scope (global block plus one host block) at
//! configuration load time and looked up per request through a radix tree
//! keyed the same way as the observability sinks.

use ferron_core::config::{ServerConfigurationBlock, ServerConfigurationDirectiveEntry};
use ferron_observability::sampler::{TraceSampler, TraceSamplingConfig};

/// Trace behaviour for one host.
#[derive(Clone, Debug)]
pub struct HttpTraceSettings {
    /// `http { trace { generate } }`: generate a trace context for requests that
    /// arrive without one.
    pub generate: bool,
    /// `http { trace { trust_request } }`: use the incoming `traceparent`,
    /// `tracestate`, and `baggage` headers as the parent trace context.
    pub trust_request: bool,
    /// `http { trace_sampling }`: which spans reach the trace sinks.
    pub sampling: TraceSamplingConfig,
}

impl Default for HttpTraceSettings {
    #[inline]
    fn default() -> Self {
        Self {
            generate: true,
            trust_request: false,
            sampling: TraceSamplingConfig::default(),
        }
    }
}

impl HttpTraceSettings {
    /// Build the sampler used by the trace sinks for this host.
    #[inline]
    pub fn sampler(&self) -> TraceSampler {
        TraceSampler::new(&self.sampling)
    }

    /// Whether `trace_sampling` drops every span.
    ///
    /// Used to stamp the `sampled` flag on a generated trace context so that
    /// upstream services and access logs agree with what Ferron exports.
    #[inline]
    pub fn default_sampled(&self) -> bool {
        !matches!(
            self.sampling.mode,
            ferron_observability::sampler::TraceSamplingMode::AlwaysOff
        )
    }

    /// The probability that `trace_sampling` applies to traces Ferron starts.
    ///
    /// Reported to backends as the W3C `ot` tracestate entry so they can
    /// extrapolate counts from partially sampled traces. `None` when the
    /// sampler is not probabilistic or the probability carries no information.
    #[inline]
    pub fn sampling_probability(&self) -> Option<f64> {
        self.sampling.mode.root_probability()
    }
}

/// Return the `http` sub-block of a configuration block, if present.
#[inline]
fn http_block(block: Option<&ServerConfigurationBlock>) -> Option<&ServerConfigurationBlock> {
    block?
        .directives
        .get("http")
        .and_then(|entries| entries.first())
        .and_then(|entry| entry.children.as_ref())
}

/// Return the `trace` sub-block nested inside an `http` block.
#[inline]
fn trace_block(http: Option<&ServerConfigurationBlock>) -> Option<&ServerConfigurationBlock> {
    http?
        .directives
        .get("trace")
        .and_then(|entries| entries.first())
        .and_then(|entry| entry.children.as_ref())
}

#[inline]
fn first_flag(block: Option<&ServerConfigurationBlock>, directive: &str) -> Option<bool> {
    block
        .and_then(|block| block.directives.get(directive))
        .and_then(|entries| entries.first())
        .map(ServerConfigurationDirectiveEntry::get_flag)
}

#[inline]
fn first_entry<'a>(
    block: Option<&'a ServerConfigurationBlock>,
    directive: &str,
) -> Option<&'a ServerConfigurationDirectiveEntry> {
    block
        .and_then(|block| block.directives.get(directive))
        .and_then(|entries| entries.first())
}

/// Resolve trace settings for one host, falling back to the global block for
/// every directive the host leaves unset.
#[inline]
pub fn resolve_trace_settings(
    global: Option<&ServerConfigurationBlock>,
    host: Option<&ServerConfigurationBlock>,
) -> HttpTraceSettings {
    let host_http = http_block(host);
    let global_http = http_block(global);

    // The host block wins per directive; `http` blocks that only exist at one
    // of the two scopes are merged rather than replaced wholesale.
    let trace = trace_block(host_http).or_else(|| trace_block(global_http));
    let sampling_host = first_entry(host_http, "trace_sampling");
    let sampling_global = first_entry(global_http, "trace_sampling");

    HttpTraceSettings {
        generate: first_flag(trace, "generate")
            .or_else(|| first_flag(trace_block(global_http), "generate"))
            .unwrap_or(true),
        trust_request: first_flag(trace, "trust_request")
            .or_else(|| first_flag(trace_block(global_http), "trust_request"))
            .unwrap_or(false),
        sampling: sampling_host
            .or(sampling_global)
            .map_or_else(TraceSamplingConfig::default, |entry| {
                ferron_observability::sampler::parse_trace_sampling_config(entry)
            }),
    }
}
