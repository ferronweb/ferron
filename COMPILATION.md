# Compilation notes

## Modules

You can decide what modules to include in your Ferron installation by copying the `ferron-build.yaml` file to `ferron-build-override.yaml`, and editing the file.

The modules are defined in the `modules` section, which is a list of modules to be included in the build. Below are the supported properties for the modules:

- **builtin** (_bool_)
  - Determines if the module is built-in (in the `ferron-modules-builtin` crate). Default: `false`
- **cargo_feature** (_String_)
  - The Cargo feature for `ferron-modules-builtin` crate to enable for this module. Used with built-in modules. Default: none
- **git** (_String_)
  - The Git repository URL for the module. Default: none
- **branch** (_String_)
  - The Git branch for the module. Used with Git repositories. Default: none
- **path** (_String_)
  - The local path (absolute paths are recommended) to the module. Default: none
- **crate** (_String_)
  - The name of the Rust crate corresponding to the module. Used with modules from Git or local paths. Default: none
- **loader** (_String_)
  - The name of the struct name that will be used to load the module (usually ends with `Loader`). The struct must have a `new` method. Default: none

For example, you can specify a module from a Git repository:

```yaml
modules:
  # ...
  - git: https://git.example.com/ferron-module-example.git
    crate: ferron-module-example
    loader: ExampleLoader
```

Or a built-in module:

```yaml
modules:
  # ...
  - builtin: true
    cargo_feature: example
    loader: ExampleLoader
```

The modules will be executed in the order they are defined in the `modules` section.

## DNS providers

You can also decide what DNS providers to include in your Ferron installation by copying the `ferron-build.yaml` file to `ferron-build-override.yaml`, and editing the file.

The providers are defined in the `dns` section, which is a list of DNS providers to be included in the build. Below are the supported properties for the DNS providers:

- **builtin** (_bool_)
  - Determines if the module is built-in (in the `ferron-modules-builtin` crate). Default: `false`
- **id** (_String_)
  - The DNS provider ID, as specified in the `provider` prop in the `auto_tls_challenge` directive in KDL configuration. Default: none
- **cargo_feature** (_String_)
  - The Cargo feature for `ferron-modules-builtin` crate to enable for this module. Used with built-in modules. Default: none
- **git** (_String_)
  - The Git repository URL for the module. Default: none
- **branch** (_String_)
  - The Git branch for the module. Used with Git repositories. Default: none
- **path** (_String_)
  - The local path (absolute paths are recommended) to the module. Default: none
- **crate** (_String_)
  - The name of the Rust crate corresponding to the module. Used with modules from Git or local paths. Default: none
- **provider** (_String_)
  - The name of the struct name that will be used to initialize the DNS provider (usually ends with `Provider`). The struct must have a `with_parameters` method. Default: none

For example, you can specify a provider from a Git repository:

```yaml
dns:
  # ...
  - git: https://git.example.com/ferron-dns-mock.git
    crate: ferron-dns-mock
    provider: MockProvider
```

Or a built-in provider:

```yaml
dns:
  # ...
  - builtin: true
    cargo_feature: mock
    provider: MockProvider
```

## Observability backends

You can also decide what observability backends to include in your Ferron installation by copying the `ferron-build.yaml` file to `ferron-build-override.yaml`, and editing the file.

The supported observability backends are defined in the `observability` section, which is a list of supported observability backends to be included in the build. Below are the supported properties for the observability backends:

- **builtin** (_bool_)
  - Determines if support for a specific observability backend is built-in (in the `ferron-observability-builtin` crate). Default: `false`
- **cargo_feature** (_String_)
  - The Cargo feature for `ferron-observability-builtin` crate to enable for support for a specific observability backend. Used with built-in support. Default: none
- **git** (_String_)
  - The Git repository URL for support for a specific observability backend. Default: none
- **branch** (_String_)
  - The Git branch for support for a specific observability backend. Used with Git repositories. Default: none
- **path** (_String_)
  - The local path (absolute paths are recommended) to support for a specific observability backend. Default: none
- **crate** (_String_)
  - The name of the Rust crate corresponding to support for a specific observability backend. Used with observability backend support crates from Git or local paths. Default: none
- **loader** (_String_)
  - The name of the struct name that will be used to load support for a specific observability backend (usually ends with `ObservabilityBackendLoader`). The struct must have a `new` method. Default: none

For example, you can specify observability backend support from a Git repository:

```yaml
observability:
  # ...
  - git: https://git.example.com/ferron-observability-example.git
    crate: ferron-observability-example
    loader: ExampleObservabilityBackendLoader
```

Or a built-in observability backend:

```yaml
observability:
  # ...
  - builtin: true
    cargo_feature: example
    loader: ExampleObservabilityBackendLoader
```
