use std::sync::Arc;

use ferron_core::config::ServerConfigurationBlock;
use ferron_core::registry::Registry;

use crate::{
    CompositeEventSink, ControlPlaneConfig, ObservabilityConfigExtractor, ObservabilityContext,
    TraceSampler, TraceSamplingConfig,
};

/// Materializes sinks from observability providers using the global config.
///
/// Extracts observability blocks from the global configuration (both explicit
/// `observability { }` blocks and alias directives like `log`, `error_log`,
/// `console_log`), looks up each provider by name, and calls `provider.execute()`
/// to materialize the sinks.
///
/// When `trace_sampling` is provided, a `TraceSampler` is attached to the
/// resulting `CompositeEventSink` so that trace events are sampled before
/// dispatching to individual sinks.
///
/// When the global config contains a `control_plane { metadata { ... } }`
/// block, the metadata key-value pairs are passed to each provider via the
/// `ObservabilityContext` so they can be included as attributes in
/// observability signals.
pub fn build_composite_sink(
    registry: &Registry,
    global_config: &Arc<ServerConfigurationBlock>,
    trace_sampling: Option<TraceSamplingConfig>,
) -> Result<Arc<CompositeEventSink>, Box<dyn std::error::Error>> {
    let mut sinks = Vec::new();

    let control_plane_metadata = ControlPlaneConfig::from_block(global_config)
        .map(|c| c.metadata);

    if let Some(observability_registry) = registry.get_provider_registry::<ObservabilityContext>() {
        // Extract observability blocks from the global config
        let extractor = ObservabilityConfigExtractor::new(global_config.as_ref());
        let observability_blocks = extractor.extract_observability_blocks()?;

        for block in observability_blocks {
            let provider_name = match block.get_value("provider").and_then(|v| v.as_str()) {
                Some(name) => name,
                None => continue, // Skip blocks without a provider
            };

            // Look up the provider by name
            let Some(provider) = observability_registry.get(provider_name) else {
                continue;
            };

            // Materialize the sink by calling provider.execute()
            let mut ctx = ObservabilityContext {
                log_config: Arc::new(block),
                sink: None,
                control_plane_metadata: control_plane_metadata.clone(),
            };
            if provider.execute(&mut ctx).is_ok() {
                if let Some(sink) = ctx.sink {
                    sinks.push(sink);
                }
            }
        }
    }

    let sampler = trace_sampling.as_ref().map(TraceSampler::new);
    Ok(Arc::new(CompositeEventSink::with_sampler(sinks, sampler)))
}
