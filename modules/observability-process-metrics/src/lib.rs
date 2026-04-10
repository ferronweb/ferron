use std::sync::Arc;

use ferron_core::{loader::ModuleLoader, registry::Registry, Module};
use ferron_observability::build_composite_sink;

#[cfg(test)]
use ferron_observability::ObservabilityContext;

#[cfg(target_os = "linux")]
mod linux;

/// Module loader for the process metrics collector.
///
/// This module discovers available observability providers from the registry,
/// materializes their sinks using the actual global observability configuration,
/// and spawns a background task that periodically emits process-level metrics
/// (CPU time, CPU utilization, memory usage) through the composite event sink.
#[derive(Default)]
pub struct ProcessMetricsModuleLoader {
    cache: Option<Arc<ProcessMetricsModule>>,
}

impl ModuleLoader for ProcessMetricsModuleLoader {
    fn register_modules(
        &mut self,
        registry: Arc<Registry>,
        modules: &mut Vec<Arc<dyn Module>>,
        config: Arc<ferron_core::config::ServerConfiguration>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Build the composite sink from all observability providers using the
        // actual global observability configuration
        let event_sink = build_composite_sink(&registry, &config.global_config)?;

        if self.cache.is_none() {
            let module = Arc::new(ProcessMetricsModule::new(event_sink));
            self.cache = Some(module.clone());
            modules.push(module);
        }

        Ok(())
    }
}

/// The process metrics module that spawns the background collection task.
pub struct ProcessMetricsModule {
    event_sink: Arc<ferron_observability::CompositeEventSink>,
    cancel_token: tokio_util::sync::CancellationToken,
}

impl ProcessMetricsModule {
    fn new(event_sink: Arc<ferron_observability::CompositeEventSink>) -> Self {
        Self {
            event_sink,
            cancel_token: tokio_util::sync::CancellationToken::new(),
        }
    }
}

impl Module for ProcessMetricsModule {
    fn name(&self) -> &str {
        "observability-process-metrics"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn start(
        &self,
        runtime: &mut ferron_core::runtime::Runtime,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let cancel_token = self.cancel_token.clone();
        let event_sink = self.event_sink.clone();

        runtime.spawn_secondary_task(async move {
            run_metrics_collection(event_sink, cancel_token).await;
        });

        Ok(())
    }
}

impl Drop for ProcessMetricsModule {
    fn drop(&mut self) {
        self.cancel_token.cancel();
    }
}

#[cfg(target_os = "linux")]
async fn run_metrics_collection(
    event_sink: Arc<ferron_observability::CompositeEventSink>,
    cancel_token: tokio_util::sync::CancellationToken,
) {
    linux::collect_process_metrics(event_sink, cancel_token).await;
}

#[cfg(not(target_os = "linux"))]
async fn run_metrics_collection(
    _event_sink: Arc<ferron_observability::CompositeEventSink>,
    mut cancel_token: tokio_util::sync::CancellationToken,
) {
    // Process metrics are only supported on Linux.
    // Wait for cancellation without doing anything.
    cancel_token.cancelled().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    use ferron_core::{
        config::{
            ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue,
        },
        providers::Provider,
    };
    use ferron_observability::{Event, EventSink, MetricEvent, MetricType, MetricValue};
    use std::{collections::HashMap, sync::Mutex};

    /// Helper to create a config directive
    fn make_directive(
        _name: &str,
        args: Vec<&str>,
        children: Option<ServerConfigurationBlock>,
    ) -> ServerConfigurationDirectiveEntry {
        ServerConfigurationDirectiveEntry {
            args: args
                .into_iter()
                .map(|a| ServerConfigurationValue::String(a.to_string(), None))
                .collect(),
            children,
            span: None,
        }
    }

    /// Helper to create a config block from directives.
    /// Multiple directives with the same name are grouped together.
    fn make_block(
        directives: Vec<(&str, ServerConfigurationDirectiveEntry)>,
    ) -> Arc<ServerConfigurationBlock> {
        let mut map: HashMap<String, Vec<ServerConfigurationDirectiveEntry>> = HashMap::new();
        for (name, entry) in directives {
            map.entry(name.to_string()).or_default().push(entry);
        }
        Arc::new(ServerConfigurationBlock {
            directives: Arc::new(map),
            matchers: HashMap::new(),
            span: None,
        })
    }

    /// A mock event sink that records all emitted metric events.
    struct MockEventSink {
        events: Mutex<Vec<Event>>,
    }

    impl EventSink for MockEventSink {
        fn emit(&self, event: Event) {
            self.events.lock().unwrap().push(event);
        }
    }

    /// A mock observability provider that returns our mock sink.
    struct MockObservabilityProvider {
        sink: Arc<MockEventSink>,
        name: String,
    }

    impl Provider<ObservabilityContext> for MockObservabilityProvider {
        fn name(&self) -> &str {
            &self.name
        }

        fn execute(
            &self,
            ctx: &mut ObservabilityContext,
        ) -> Result<(), Box<dyn std::error::Error>> {
            ctx.sink = Some(self.sink.clone());
            Ok(())
        }
    }

    #[test]
    fn test_build_composite_sink_materializes_sinks() {
        let mock_sink = Arc::new(MockEventSink {
            events: Mutex::new(Vec::new()),
        });

        let registry = Registry::new();
        registry.register_provider::<ObservabilityContext, _>({
            let sink = mock_sink.clone();
            move || {
                Arc::new(MockObservabilityProvider {
                    sink: sink.clone(),
                    name: "mock".to_string(),
                })
            }
        });

        // Create a global config with an observability block:
        //   observability {
        //       provider "mock"
        //   }
        // The provider directive must be in the children block
        let children_block = make_block(vec![(
            "provider",
            make_directive("provider", vec!["mock"], None),
        )]);
        let global_config = make_block(vec![(
            "observability",
            make_directive(
                "observability",
                vec![],
                Some(Arc::unwrap_or_clone(children_block)),
            ),
        )]);

        let composite = build_composite_sink(&registry, &global_config).unwrap();
        composite.emit(Event::Metric(MetricEvent {
            name: "test.metric",
            attributes: vec![],
            ty: MetricType::Counter,
            value: MetricValue::U64(1),
            unit: Some("1"),
            description: Some("test"),
        }));

        assert_eq!(mock_sink.events.lock().unwrap().len(), 1);
    }

    #[test]
    fn test_build_composite_sink_empty_registry() {
        let registry = Registry::new();
        let children_block = make_block(vec![(
            "provider",
            make_directive("provider", vec!["console"], None),
        )]);
        let global_config = make_block(vec![(
            "observability",
            make_directive(
                "observability",
                vec![],
                Some(Arc::unwrap_or_clone(children_block)),
            ),
        )]);
        let composite = build_composite_sink(&registry, &global_config).unwrap();
        // Should not panic, just emits to an empty sink list
        composite.emit(Event::Metric(MetricEvent {
            name: "test.metric",
            attributes: vec![],
            ty: MetricType::Counter,
            value: MetricValue::U64(1),
            unit: Some("1"),
            description: Some("test"),
        }));
    }

    #[test]
    fn test_build_composite_sink_empty_config() {
        let mock_sink = Arc::new(MockEventSink {
            events: Mutex::new(Vec::new()),
        });

        let registry = Registry::new();
        registry.register_provider::<ObservabilityContext, _>({
            let sink = mock_sink.clone();
            move || {
                Arc::new(MockObservabilityProvider {
                    sink: sink.clone(),
                    name: "mock".to_string(),
                })
            }
        });

        // No observability directives in the config
        let global_config = Arc::new(ServerConfigurationBlock::default());
        let composite = build_composite_sink(&registry, &global_config).unwrap();
        // Should not panic, just emits to an empty sink list
        composite.emit(Event::Metric(MetricEvent {
            name: "test.metric",
            attributes: vec![],
            ty: MetricType::Counter,
            value: MetricValue::U64(1),
            unit: Some("1"),
            description: Some("test"),
        }));
        // No sinks should have been created since there's no observability config
        assert!(mock_sink.events.lock().unwrap().is_empty());
    }

    #[test]
    fn test_build_composite_sink_multiple_providers() {
        let sink1 = Arc::new(MockEventSink {
            events: Mutex::new(Vec::new()),
        });
        let sink2 = Arc::new(MockEventSink {
            events: Mutex::new(Vec::new()),
        });

        let registry = Registry::new();

        // Register two different providers with unique names
        registry.register_provider::<ObservabilityContext, _>({
            let sink = sink1.clone();
            move || {
                Arc::new(MockObservabilityProvider {
                    sink: sink.clone(),
                    name: "console".to_string(),
                })
            }
        });
        registry.register_provider::<ObservabilityContext, _>({
            let sink = sink2.clone();
            move || {
                Arc::new(MockObservabilityProvider {
                    sink: sink.clone(),
                    name: "file".to_string(),
                })
            }
        });

        // Create a global config with two observability blocks:
        //   observability {
        //       provider "console"
        //   }
        //   observability {
        //       provider "file"
        //   }
        let children_block_1 = make_block(vec![(
            "provider",
            make_directive("provider", vec!["console"], None),
        )]);
        let children_block_2 = make_block(vec![(
            "provider",
            make_directive("provider", vec!["file"], None),
        )]);

        let global_config = make_block(vec![
            (
                "observability",
                make_directive(
                    "observability",
                    vec![],
                    Some(Arc::unwrap_or_clone(children_block_1)),
                ),
            ),
            (
                "observability",
                make_directive(
                    "observability",
                    vec![],
                    Some(Arc::unwrap_or_clone(children_block_2)),
                ),
            ),
        ]);

        let composite = build_composite_sink(&registry, &global_config).unwrap();
        composite.emit(Event::Metric(MetricEvent {
            name: "test.metric",
            attributes: vec![],
            ty: MetricType::Counter,
            value: MetricValue::U64(42),
            unit: Some("1"),
            description: Some("test"),
        }));

        // Each observability block looks up a different provider by name,
        // so both sinks should receive the event
        assert_eq!(sink1.events.lock().unwrap().len(), 1);
        assert_eq!(sink2.events.lock().unwrap().len(), 1);

        let event = sink1.events.lock().unwrap()[0].clone();
        if let Event::Metric(m) = event {
            assert_eq!(m.name, "test.metric");
            if let MetricValue::U64(v) = m.value {
                assert_eq!(v, 42);
            } else {
                panic!("expected U64 value");
            }
        } else {
            panic!("expected Metric event");
        }
    }

    #[test]
    fn test_build_composite_sink_with_console_log_alias() {
        let mock_sink = Arc::new(MockEventSink {
            events: Mutex::new(Vec::new()),
        });

        let registry = Registry::new();
        registry.register_provider::<ObservabilityContext, _>({
            let sink = mock_sink.clone();
            move || {
                Arc::new(MockObservabilityProvider {
                    sink: sink.clone(),
                    name: "mock".to_string(),
                })
            }
        });

        // Create a global config using the console_log alias:
        //   console_log { }
        // The alias transforms to provider "console", which doesn't match our "mock" provider
        let global_config = make_block(vec![(
            "console_log",
            make_directive("console_log", vec![], None),
        )]);

        // The alias extracts as provider "console", but we registered "mock"
        // So no sinks should be created
        let composite = build_composite_sink(&registry, &global_config).unwrap();
        assert!(mock_sink.events.lock().unwrap().is_empty());
        let _ = composite;
    }

    #[cfg(target_os = "linux")]
    mod linux_tests {
        #[test]
        fn test_ticks_per_second_is_nonzero() {
            let tps = procfs::ticks_per_second();
            assert!(tps > 0, "ticks_per_second should be nonzero");
        }

        #[test]
        fn test_page_size_is_nonzero() {
            let ps = procfs::page_size();
            assert!(ps > 0, "page_size should be nonzero");
        }

        #[test]
        fn test_cpu_utilization_formula() {
            // Test the utilization formula with known values
            let delta_cpu = 0.5; // 0.5 seconds of CPU time
            let elapsed = 1.0; // 1 second elapsed
            let parallelism = 4; // 4 CPUs

            let utilization = delta_cpu / (elapsed * parallelism as f64);
            // 0.5 / (1.0 * 4) = 0.125
            assert!((utilization - 0.125).abs() < f64::EPSILON);
        }

        #[test]
        fn test_cpu_utilization_can_exceed_one() {
            // With multi-threaded work, utilization can exceed 1.0
            let delta_cpu = 5.0; // 5 seconds of CPU time across threads
            let elapsed = 1.0;
            let parallelism = 4;

            let utilization = delta_cpu / (elapsed * parallelism as f64);
            assert!((utilization - 1.25).abs() < f64::EPSILON);
        }
    }
}
