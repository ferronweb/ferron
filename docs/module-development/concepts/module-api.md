---
title: "Module API"
description: "Introduction to the Ferron 3 module API: ModuleLoader, Stage, Provider, Module, ConfigurationAdapter and helpers."
---

A Ferron module implements a few traits from `ferron-core` and registers them. The sections below list each trait, its purpose, and the source file with full details.

## `ModuleLoader`

Trait: `ferron_core::loader::ModuleLoader` (file: `core/src/loader.rs`).

`ModuleLoader` is the registration entry point. Derive `Default` and override only the methods the module needs. Every method has a no-op default.

Typical overrides:

- `register_stages`: call `registry.with_stage::<C, _>(|| Arc::new(MyStage))`.
- `register_providers`: call `registry.with_provider::<C, _>(|| Arc::new(MyProvider))`.
- `register_directives`: call `registry.register(Directive { ... }, subblock)`.
- `register_*_validators`: push validators or insert scoped validators.
- `register_configuration_adapters`: insert `Box<dyn ConfigurationAdapter>`.
- `register_modules`: read `ServerConfiguration`, create `Arc<dyn Module>`, and push it into `modules`. Return `Err` to abort start-up.

## `Stage<C>`

Trait: `ferron_core::pipeline::Stage<C>` (file: `core/src/pipeline.rs`).

Stages form the request pipeline. `C` is the context type, for example `HttpContext`, `HttpFileContext`, or `HttpErrorContext` (types in `types/http`).

- `name() -> &str`: unique within `C`.
- `constraints() -> Vec<StageConstraint>`: `Before` and `After` ordering.
- `is_applicable(config: Option<&ServerConfigurationBlock>) -> bool`: called when building with `StageRegistry::build_with_config`. Return `false` to omit the stage when its directive is absent.
- `run(&self, ctx: &mut C) -> Result<bool, PipelineError>`: forward pass. `Ok(true)` continues, `Ok(false)` stops gracefully, and `Err` stops with an error.
- `run_inverse(&self, ctx: &mut C) -> Result<(), PipelineError>`: reverse pass for cleanup and response modification.

Per-request state goes into `ctx.extensions` (`TypeMap`). Threads share stages, so do not store request state in `self`.

## `Provider<C>` and `DnsClient`, `TlsResolver`, `EventSink`

Traits: `ferron_core::providers::Provider<C>` (`core/src/providers.rs`), plus domain types in `types/tls`, `types/dns`, and `types/observability`.

Ferron discovers providers by type and name at runtime:

```rust
registry.with_provider::<TlsContext, _>(|| Arc::new(MyTlsProvider));
let provider = registry.get_provider_registry::<TlsContext>().unwrap().get("my_tls").unwrap();
```

Common families:

- `Provider<TlsContext>`: `execute` sets `ctx.resolver: Option<Arc<dyn TlsResolver>>`.
- `Provider<DnsContext>`: `execute` sets `ctx.client: Option<Arc<dyn DnsClient>>`.
- `Provider<ObservabilityContext>`: `execute` sets `ctx.sink: Option<Arc<dyn EventSink>>`.
- `Provider<LogFormatterContext>` and `Provider<ApplicationLogFormatterContext>`: `execute` sets `ctx.output: Option<String>`.

Scoped validators use `config_validator_scoped_key!(ns, name)` with the namespaces `tls`, `dns`, `observability`, `logformat`, and so on. The key must match `Provider::name()`.

## `Module`

Trait: `ferron_core::Module` (`core/src/lib.rs`).

`Module` represents long-lived components. `Module::start(&self, runtime: &mut Runtime)` spawns tasks. Use:

- `runtime.spawn_primary_task(|| Box::pin(async { ... }))` for per-CPU zincio tasks.
- `runtime.spawn_secondary_task(async { ... })` for tokio tasks.
- `runtime.spawn_primary_task_on(idx, ...)` to pin a task to a specific CPU.

Read `SHUTDOWN_TOKEN` and `RELOAD_TOKEN` (from `core/src/shutdown.rs`) to handle a graceful stop.

## `ConfigurationAdapter` and `ConfigurationWatcher`

Traits: `ferron_core::config::adapter::{ConfigurationAdapter, ConfigurationWatcher}` (`core/src/config/adapter.rs`).

- `adapt(&self, params: &HashMap<String,String>) -> AdaptResult` returns `(ServerConfiguration, Box<dyn ConfigurationWatcher>, ConfigurationMetadata)`.
- `file_extension() -> Vec<&'static str>` selects the adapter by file suffix.
- The watcher implements `watch(&mut self) -> Future`.

Drift detection uses `ConfigurationMetadata` (`config_hash`, `config_mtime`, `config_files`) and the `ADMIN_METRICS.config_drift` gauge.

## Configuration helpers

- `core/src/config/mod.rs`: `ServerConfiguration`, `ServerConfigurationBlock`, `ServerConfigurationValue`, `LayeredConfiguration`, and `Variables`.
- `core/src/config/validator.rs`: `ConfigurationValidator`, `ConfigurationValidatorContext`, `validate_scoped_block!`, and `entry_span`.
- `core/src/config/macros`: `validate_directive!`, `validate_nested!`, and `config_validator_scoped_key!`.
- `types/http`: `HttpContext`, `HttpRequest`, `HttpResponse`, and `HttpFileContext`.

## Observability

Ferron has two observability channels. Use the right one for each situation.

### Application logging macros

`ferron_core::log_info!`, `log_warn!`, `log_error!`, and `log_debug!` write to stdout (or the Windows Event Log). These calls are synchronous and unstructured. Use them for server-infrastructure events: startup, shutdown, TLS configuration, and file rotation errors.

```rust
ferron_core::log_info!("Listening on port {}", port);
ferron_core::log_error!("TLS handshake failed: {}", err);
```

The macros check a global level guard. The lowest level is `Debug`.

### Structured event system

Request processing uses `Event` values emitted through `ctx.events` (`CompositeEventSink`) on `HttpContext`.

A `CompositeEventSink` wraps multiple `EventSink` implementations. It applies trace sampling before dispatch and optimizes the one-sink and zero-sink cases. Use `CompositeEventSink::default()` for tests and no-op sinks.

The `Event` enum has these variants:

- `Event::Log`: a log event.
- `Event::Metric`: a metric event.
- `Event::Trace`: a trace event, which can be the start or the end of a span.
- `Event::Access`: an access log event.

## Where to read more

- The source files listed above. Each has module-level docs and examples.
- Generated Rust docs via `cargo doc --no-deps`. See [Obtaining API docs](/docs/module-development/guides/obtaining-api-docs).
- Runnable examples in the `ferron3-example-modules` repository at `https://github.com/ferronweb/ferron3-example-modules`. Each crate is about 150 lines and has inline comments.
