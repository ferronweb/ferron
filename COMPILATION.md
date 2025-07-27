# Compilation notes

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
  - The name of the struct name that will be used to load the module (usually ends with `Loader`). Default: none

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
