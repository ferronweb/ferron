---
title: "Creating a Ferron 3 module"
description: "Step-by-step guide to create a Ferron 3 ModuleLoader crate and wire it into a custom binary."
---

Ferron modules are Rust library crates that implement `ModuleLoader`. You add
them to a custom binary that depends on `ferron-entrypoint`. Ferron then calls
your loader during start-up to register stages, providers, validators, and
directives.

## 1. Create the crate

```bash
cargo new ferron-http-example --lib
cd ferron-http-example
```

`Cargo.toml`:

```toml
[package]
name = "ferron-http-example"
version = "0.1.0"
edition = "2021"

[dependencies]
async-trait = "0.1"
ferron-core = { git = "https://github.com/ferronweb/ferron", branch = "3.x" }
ferron-http = { git = "https://github.com/ferronweb/ferron", branch = "3.x" }
```

> [!tip]
> For local development inside a clone of `ferron`, add a patch that redirects
> the git source to the local checkout so `cargo build` works offline. See the
> example modules repository for a complete workspace example:
> `https://github.com/ferronweb/ferron3-example-modules`.

## 2. Implement `ModuleLoader`

```rust
use std::sync::Arc;
use async_trait::async_trait;
use ferron_core::loader::ModuleLoader;
use ferron_core::pipeline::{Stage, PipelineError};
use ferron_core::registry::{RegistryBuilder, StageConstraint};
use ferron_http::HttpContext;

struct ExampleStage;

#[async_trait(?Send)]
impl Stage<HttpContext> for ExampleStage {
    fn name(&self) -> &str { "example" }
    fn constraints(&self) -> Vec<StageConstraint> {
        vec![StageConstraint::Before("reverse_proxy".into())]
    }
    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        // read ctx.configuration, inspect ctx.req, set ctx.res
        Ok(true)
    }
}

#[derive(Default)]
pub struct ExampleModuleLoader;

impl ModuleLoader for ExampleModuleLoader {
    fn register_stages(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        registry.with_stage::<HttpContext, _>(|| Arc::new(ExampleStage))
    }
}
```

### Registration order

Ferron calls `ModuleLoader` methods in this order:

1. `register_configuration_adapters`
2. `register_per_protocol_configuration_blocks`
3. `register_global_configuration_validators`
4. `register_per_protocol_configuration_validators`
5. `register_scoped_configuration_validators`
6. `register_stages`
7. `register_providers`
8. `register_directives`
9. `register_modules` (creates `Module` instances, may read finalized config)

All methods have default no-op impls. Override only what you need.

## 3. Add directives and validators

Register editor metadata and validation:

```rust
use ferron_core::directives::{Directive, DirectiveRegistry, DirectiveSubblock};
use ferron_core::config::validator::{ConfigurationValidator, ConfigurationValidationError};

impl ModuleLoader for ExampleModuleLoader {
    fn register_directives(&mut self, registry: &mut DirectiveRegistry) {
        registry.register(
            Directive {
                name: "example_directive",
                usage: "example_directive <value>",
                description: "Example directive that does something.",
                applicable_protocols: Some(&["http"]),
                global_only: false,
                subblock_link: None,
            },
            DirectiveSubblock::default(),
        );
    }
    fn register_global_configuration_validators(
        &mut self,
        registry: &mut Vec<Box<dyn ConfigurationValidator>>,
    ) {
        registry.push(Box::new(ExampleValidator));
    }
}

struct ExampleValidator;
impl ConfigurationValidator for ExampleValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ConfigurationValidationError> {
        ferron_core::validate_directive!(
            config, ctx.used_directives, example_directive,
            optional args(1) => [ferron_core::config::ServerConfigurationValue::String(_, _)],
            {}
        );
        Ok(())
    }
}
```

Mark directives as used so `UnknownDirective` diagnostics are accurate. Use
`validate_directive!` and `validate_nested!` helpers from `ferron-core`.

## 4. Wire the module into a binary

Create a binary crate that depends on `ferron-entrypoint` and your module:

```toml
[dependencies]
ferron-entrypoint = { git = "https://github.com/ferronweb/ferron", branch = "3.x", features = ["profile-default"] }
ferron-http-example = { path = "../ferron-http-example" }
```

`src/main.rs`:

```rust
fn main() {
    ferron_entrypoint::init();
    let mut profile = ferron_entrypoint::default_profile();
    profile.push(Box::new(ferron_http_example::ExampleModuleLoader::default()));
    ferron_entrypoint::main(profile);
}
```

> [!note]
> The binary must recompile when the module list changes. Modules are
> statically linked. Use `cargo build -r -p your-binary` and run with
> `./target/release/your-binary run -c ferron.conf`.

## 5. Test the module

- Unit tests: test `Stage::run` with a synthetic `HttpContext` (see
  `ferron-http-header-append` tests in the example repo).
- `ferron validate -c ferron.conf`: checks validation without starting.
- `ferron directives | jq .`: confirms directives appear.
- E2E: build an image with `e2e/Dockerfile.test` and run `testcontainers`
  tests that issue real HTTP requests.

## Choosing the right extension point

| Need                                                    | Use                               |
| ------------------------------------------------------- | --------------------------------- |
| Per-request logic                                       | `Stage<C>`                        |
| Long-lived listener or background task                  | `Module` + `Runtime`              |
| Pluggable backend (TLS, DNS, observability, log format) | `Provider<C>`                     |
| New config source                                       | `ConfigurationAdapter`            |
| Editor support                                          | `Directive` + `DirectiveRegistry` |
| Config errors / warnings                                | `ConfigurationValidator`          |

## Next steps

- Read [Module API](/docs/module-development/concepts/module-api) for trait details and helpers.
- Check [Examples](/docs/module-development/guides/examples) for runnable crates to copy.
- See [Naming conventions](/docs/module-development/guides/naming-conventions) for directive and crate names.
