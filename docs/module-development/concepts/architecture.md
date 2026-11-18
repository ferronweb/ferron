---
title: "Ferron 3 architecture"
description: "How Ferron 3 starts, orders pipeline stages, and runs primary and secondary runtimes."
---

The Ferron core is small. It defines the extension points, and modules plug into them. The sections below follow the flow from start-up to request handling.

## Start-up flow

1. `ferron_entrypoint::init()` sets the global allocator and installs the panic hook.
2. `ferron_entrypoint::default_profile()` returns `Vec<Box<dyn ModuleLoader>>` with all built-in modules. For a custom binary, start from this list and add your loaders.
3. `ferron_entrypoint::main(profile)` parses CLI arguments (`run`, `validate`, `adapt`, `directives`), loads the configuration through the selected `ConfigurationAdapter`, and runs validators.
4. Ferron builds the `Registry`: one `StageRegistry<C>` per context type and one `ProviderRegistry<P>` per provider type. `RegistryBuilder` provides `with_stage` and `with_provider` for registration.
5. `ModuleLoader::register_modules` creates `Module` instances and passes them the finalized `ServerConfiguration`.
6. `Module::start` receives `&mut Runtime` and spawns tasks.

If a validator or `register_modules` returns an error, Ferron exits before it opens sockets.

## Pipeline and stages

A `Pipeline<C>` is an ordered list of `Stage<C>` objects. For HTTP, `C` is `HttpContext`. The server builds the pipeline once per host or per location from the `StageRegistry`.

Stages declare ordering with `StageConstraint::Before` and `StageConstraint::After`. The registry sorts them with Kahn's algorithm (topological sort). A cycle panics with a diagnostic that names the conflict.

Execution:

- `run` runs in order. `Ok(true)` continues, `Ok(false)` stops the forward pass gracefully with no error, and `Err(PipelineError)` stops with an error.
- After the forward pass, `run_inverse` runs in reverse order for every stage that returned `Ok(true)` or `Ok(false)`. Stages modify the response or emit access logs here. If any `run_inverse` returns `Err`, the pipeline stops.

Hooks (`StageHooks`) run before and after each stage. Ferron uses them to emit per-stage trace spans without coupling `Pipeline` to observability code.

`is_applicable` controls whether a stage appears at all. Ferron calls it with an `Option<&ServerConfigurationBlock>` that merges all host blocks. If no block uses the stage directive, the stage is omitted. This keeps the pipeline lean.

## Providers

Providers are pluggable services that Ferron discovers by type and name at runtime. They implement `Provider<C>` and are registered with `RegistryBuilder::with_provider`. At runtime, a consumer calls `registry.get_provider_registry::<C>()` and `registry.get("name")` to create an instance through the factory.

Common provider families:

- `Provider<TlsContext>` and `TlsResolver`: certificate resolution for a hostname.
- `Provider<DnsContext>` and `DnsClient`: DNS record management for ACME.
- `Provider<ObservabilityContext>` and `EventSink`: log, metric, and trace sinks.
- `Provider<LogFormatterContext>`: access and application log formatting.

Providers do not depend on each other at compile time. The registry is the only shared type, so any module can use a provider without importing its crate.

## Observability

Ferron has two observability channels with different purposes.

### Application logging macros

`ferron_core::log_info!`, `log_warn!`, `log_error!`, and `log_debug!` write to stdout (or the Windows Event Log). These calls are synchronous and unstructured. Use them for server-infrastructure events: startup, shutdown, TLS configuration, file rotation failures, and daemon lifecycle.

These macros check a global level guard before formatting. The lowest level is `Debug`. There is no `Trace` level. The logger initializes once at startup through `logging::init_stdio_logger(level)`.

### Structured event system

Request processing uses the structured event system in `types/observability`. Modules emit `Event` values through `ctx.events` (`CompositeEventSink`) on `HttpContext`. The `Event` enum has four variants:

- `Event::Access(Arc<dyn AccessEvent>)`: a structured access log entry that uses a visitor pattern. The visitor receives typed fields (strings, integers, and booleans) without allocating a formatted string.
- `Event::Log(LogEvent)`: a structured log with level, message, target, key-value attributes, and optional trace context.
- `Event::Metric(MetricEvent)`: a numeric metric with attributes and an optional unit. The metric can be a counter, gauge, up/down counter, or histogram.
- `Event::Trace(TraceEvent)`: a distributed trace span start or end, with parent, attributes, links, and control plane metadata.

Each event carries an optional `EventTraceContext` that links the event to a W3C traceparent. The trace sampler evaluates this context before dispatch. Events that are not sampled are dropped without a log entry.

## Dual runtime

Ferron uses two runtimes:

- Primary runtime: one zincio thread per CPU, pinned through `core_affinity`, with optional `io_uring` on Linux (`RuntimeSettings::io_uring_enabled`). Use `spawn_primary_task` for TCP accept loops and connection handling. The factory closure is called once per primary thread, so it can hold thread-local state.
- Secondary runtime: a standard tokio multi-thread pool with `available_parallelism / 2` threads, minimum 1. Use `spawn_secondary_task` for background work: metrics, certificate renewal, and custom servers that do not need per-CPU threads.

`Module::start` receives `&mut Runtime` and usually spawns one task of either kind. Long-lived tasks should watch `SHUTDOWN_TOKEN` and `RELOAD_TOKEN` for a graceful stop.

## Configuration lifecycle

- Adapters (`ConfigurationAdapter`) load `ServerConfiguration` from a source (file, database, or API). They return a `ConfigurationWatcher` plus `ConfigurationMetadata` (hash, modification time, and files). Ferron selects the adapter through `--config-adapter` or by file extension.
- Validators (`ConfigurationValidator`) run against each block. Scoped validators (`config_validator_scoped_key!(ns, name)`) run when a block selects a provider. For example, `tls { provider selfsigned }` runs the `tls.selfsigned` validator.
- At runtime, handlers read `LayeredConfiguration`. It merges global, host, and location blocks, and a child value overrides a parent value.

## See also

- `core/src/loader.rs`: `ModuleLoader` trait and call order.
- `core/src/pipeline.rs`: `Stage`, `Pipeline`, and `StageHooks`.
- `core/src/registry.rs`: `Registry`, `StageRegistry`, and `ProviderRegistry`.
- `core/src/runtime.rs`: `Runtime` and `RuntimeSettings`.
- `core/src/logging.rs`: application logging macros and `AppLogger`.
- `types/observability/src/sink.rs`: `EventSink` trait and `CompositeEventSink`.
- `types/observability/src/event.rs`: `Event` enum and event types.
- `types/observability/src/provider.rs`: `ObservabilityContext`.
