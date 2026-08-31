---
title: "Module API"
description: "Introduction to the Ferron 3 module API: ModuleLoader, Stage, Provider, Module, ConfigurationAdapter and helpers."
---

This page summarizes the traits you implement when you write a Ferron module
and points to the source for full details.

## `ModuleLoader`

Trait: `ferron_core::loader::ModuleLoader` (file: `core/src/loader.rs`).

`ModuleLoader` is the registration entry point. You derive `Default` and
override only the methods your module needs. All methods have no-op defaults.

Typical overrides:

- `register_stages`: call `registry.with_stage::<C, _>(|| Arc::new(MyStage))`.
- `register_providers`: call `registry.with_provider::<C, _>(|| Arc::new(MyProvider))`.
- `register_directives`: call `registry.register(Directive { ... }, subblock)`.
- `register_*_validators`: push validators or insert scoped validators.
- `register_configuration_adapters`: insert `Box<dyn ConfigurationAdapter>`.
- `register_modules`: read `ServerConfiguration`, create `Arc<dyn Module>`,
  push into `modules`. Return `Err` to abort start-up.

## `Stage<C>`

Trait: `ferron_core::pipeline::Stage<C>` (file: `core/src/pipeline.rs`).

Stages form the request pipeline. `C` is the context type, e.g.
`HttpContext`, `HttpFileContext`, or `HttpErrorContext` (types in
`types/http`).

- `name() -> &str`: unique within `C`.
- `constraints() -> Vec<StageConstraint>`: `Before` / `After` ordering.
- `is_applicable(config: Option<&ServerConfigurationBlock>) -> bool`: called
  when building with `StageRegistry::build_with_config`. Return `false` to
  omit the stage when its directive is absent.
- `run(&self, ctx: &mut C) -> Result<bool, PipelineError>`: forward pass.
  `Ok(true)` continues, `Ok(false)` stops gracefully, `Err` stops with error.
- `run_inverse(&self, ctx: &mut C) -> Result<(), PipelineError>`: reverse
  pass for cleanup and response modification.

Per-request state goes into `ctx.extensions` (`TypeMap`). Stages are shared
across threads, so do not store request state in `self`.

## `Provider<C>` and `DnsClient`, `TlsResolver`, `EventSink`

Traits: `ferron_core::providers::Provider<C>` (`core/src/providers.rs`),
plus domain types in `types/tls`, `types/dns`, `types/observability`.

Providers are discovered by type and name at runtime:

```rust
registry.with_provider::<TlsContext, _>(|| Arc::new(MyTlsProvider));
let provider = registry.get_provider_registry::<TlsContext>().unwrap().get("my_tls").unwrap();
```

Common families:

- `Provider<TlsContext>` where `execute` sets `ctx.resolver: Option<Arc<dyn TlsResolver>>`.
- `Provider<DnsContext>` where `execute` sets `ctx.client: Option<Arc<dyn DnsClient>>`.
- `Provider<ObservabilityContext>` where `execute` sets `ctx.sink: Option<Arc<dyn EventSink>>`.
- `Provider<LogFormatterContext>` / `Provider<ApplicationLogFormatterContext>` where
  `execute` sets `ctx.output: Option<String>`.

Scoped validators use `config_validator_scoped_key!(ns, name)` with namespaces
`tls`, `dns`, `observability`, `logformat`, etc. The key must match
`Provider::name()`.

## `Module`

Trait: `ferron_core::Module` (`core/src/lib.rs`).

Long-lived components. `Module::start(&self, runtime: &mut Runtime)` spawns
tasks. Use:

- `runtime.spawn_primary_task(|| Box::pin(async { ... }))` for per-CPU zincio tasks.
- `runtime.spawn_secondary_task(async { ... })` for tokio tasks.
- `runtime.spawn_primary_task_on(idx, ...)` to pin to a specific CPU.

Read `SHUTDOWN_TOKEN` / `RELOAD_TOKEN` (from `core/src/shutdown.rs`) to
handle graceful stop.

## `ConfigurationAdapter` / `ConfigurationWatcher`

Traits: `ferron_core::config::adapter::{ConfigurationAdapter, ConfigurationWatcher}`
(`core/src/config/adapter.rs`).

- `adapt(&self, params: &HashMap<String,String>) -> AdaptResult` returns
  `(ServerConfiguration, Box<dyn ConfigurationWatcher>, ConfigurationMetadata)`.
- `file_extension() -> Vec<&'static str>` selects the adapter by file suffix.
- The watcher implements `watch(&mut self) -> Future` and `check_drift`.

Drift detection uses `ConfigurationMetadata` (`config_hash`, `config_mtime`,
`config_files`) and the `ADMIN_METRICS.config_drift` gauge.

## Configuration helpers

- `core/src/config/mod.rs`: `ServerConfiguration`, `ServerConfigurationBlock`,
  `ServerConfigurationValue`, `LayeredConfiguration`, `Variables`.
- `core/src/config/validator.rs`: `ConfigurationValidator`, `ConfigurationValidatorContext`,
  `validate_scoped_block!`, `entry_span`.
- `core/src/config/macros`: `validate_directive!`, `validate_nested!`,
  `config_validator_scoped_key!`.
- `types/http`: `HttpContext`, `HttpRequest`, `HttpResponse`, `HttpFileContext`.

## Where to read more

- Source files listed above (each has module-level docs and examples).
- Generated Rust docs via `cargo doc --no-deps` (see
  [Obtaining API docs](/docs/v3/module-development/obtaining-api-docs)).
- Runnable examples in `https://github.com/ferronweb/ferron3-example-modules`
  (each crate is ~150 lines and has inline comments).
