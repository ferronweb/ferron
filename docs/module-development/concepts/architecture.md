---
title: "Ferron 3 architecture"
description: "How Ferron 3 starts, orders pipeline stages, and runs primary and secondary runtimes."
---

Ferron builds on a small core. The core defines extension points and the server
wires modules through them. This page describes the flow from start-up to
request handling.

## Start-up flow

1. `ferron_entrypoint::init()` sets the global allocator and installs the panic
   hook.
2. `ferron_entrypoint::default_profile()` returns `Vec<Box<dyn ModuleLoader>>`
   with all built-in modules. For a custom binary you start from this list and
   add your loaders.
3. `ferron_entrypoint::main(profile)` parses CLI args (`run`, `validate`,
   `adapt`, `directives`), loads configuration via the selected
   `ConfigurationAdapter`, and runs validators.
4. Ferron builds the `Registry`: one `StageRegistry<C>` per context type and
   one `ProviderRegistry<P>` per provider type. `RegistryBuilder` provides
   `with_stage` and `with_provider` for registration.
5. `ModuleLoader::register_modules` creates `Module` instances and passes them
   the finalized `ServerConfiguration`.
6. `Module::start` receives `&mut Runtime` and spawns tasks.

If any validator or `register_modules` returns an error, Ferron exits before
it opens sockets.

## Pipeline and stages

A `Pipeline<C>` is an ordered list of `Stage<C>` objects. For HTTP, `C` is
`HttpContext`. The server builds the pipeline once per host (or per location)
from the `StageRegistry`.

Stages declare ordering with `StageConstraint::Before` and
`StageConstraint::After`. The registry sorts them via Kahn's algorithm
(topological sort). A cycle panics with a diagnostic that names the conflict.

Execution:

- `run` runs in order. `Ok(true)` continues, `Ok(false)` stops the forward
  pass gracefully (no error), `Err(PipelineError)` stops with an error.
- After the forward pass, `run_inverse` runs in reverse order for every stage
  that returned `Ok(true)` or `Ok(false)`. This is where stages modify the
  response or emit access logs. If any `run_inverse` returns `Err`, the pipeline
  stops.

Hooks (`StageHooks`) run before and after each stage. Ferron uses them to emit
per-stage trace spans without coupling `Pipeline` to observability code.

`is_applicable` controls whether a stage appears at all. Ferron calls it with
`Option<&ServerConfigurationBlock>` that merges all host blocks. If no block
uses the stage's directive, the stage is omitted. This keeps the pipeline lean.

## Providers

Providers are pluggable services discovered by type and name at runtime. They
implement `Provider<C>` and are registered with
`RegistryBuilder::with_provider`. At runtime a consumer calls
`registry.get_provider_registry::<C>()` and `registry.get("name")` to create an
instance via the factory.

Common provider families:

- `Provider<TlsContext>` / `TlsResolver`: certificate resolution for a hostname.
- `Provider<DnsContext>` / `DnsClient`: DNS record management for ACME.
- `Provider<ObservabilityContext>` / `EventSink`: log, metric, and trace sinks.
- `Provider<LogFormatterContext>`: access and application log formatting.

Providers do not depend on each other at compile time. The registry is the
only shared type, so any module can use a provider without importing its crate.

## Dual runtime

Ferron uses two runtimes:

- **Primary runtime**: one zincio thread per CPU, pinned via `core_affinity`,
  optionally with `io_uring` on Linux (`RuntimeSettings::io_uring_enabled`).
  Use `spawn_primary_task` for TCP accept loops and connection handling.
  The factory closure is called once per primary thread, so you can hold
  thread-local state.
- **Secondary runtime**: standard tokio multi-thread pool
  (`available_parallelism / 2` threads, minimum 1). Use
  `spawn_secondary_task` for background work: metrics, cert renewal, custom
  servers that do not need per-CPU threads.

`Module::start` receives `&mut Runtime` and typically spawns one task of
either kind. Long-lived tasks should watch `SHUTDOWN_TOKEN` / `RELOAD_TOKEN`
for graceful stop.

## Configuration lifecycle

- Adapters (`ConfigurationAdapter`) load `ServerConfiguration` from a source
  (file, DB, API) and return a `ConfigurationWatcher` plus `ConfigurationMetadata`
  (hash, mtime, files). Ferron selects the adapter via `--config-adapter` or
  by file extension.
- Validators (`ConfigurationValidator`) run against each block. Scoped
  validators (`config_validator_scoped_key!(ns, name)`) run when a block
  selects a provider (e.g. `tls { provider selfsigned }` runs the
  `tls.selfsigned` validator).
- At runtime, handlers read `LayeredConfiguration`, which merges global + host + location blocks with child-over-parent semantics.

## See also

- `core/src/loader.rs`: `ModuleLoader` trait and call order.
- `core/src/pipeline.rs`: `Stage`, `Pipeline`, `StageHooks`.
- `core/src/registry.rs`: `Registry`, `StageRegistry`, `ProviderRegistry`.
- `core/src/runtime.rs`: `Runtime` and `RuntimeSettings`.
