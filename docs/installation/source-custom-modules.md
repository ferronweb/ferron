---
title: "Building Ferron 3 from source (custom modules)"
description: "How to build a custom Ferron 3 binary with external or community-developed modules."
---

While the default `ferron` binary includes a broad set of modules, you can build your own custom version of Ferron to include external modules, community-developed extensions, or to exclude default features for a smaller binary footprint.

## Prerequisites

- **Rust toolchain** — Install from [rustup.rs](https://rustup.rs/).
- **Access to Ferron source** — You will need the Ferron repository or reference `ferron-entrypoint` via git/path.

## Creating a custom binary project

To build a custom binary, create a new Rust binary crate that depends on `ferron-entrypoint`.

### 1. Initialize the project

```bash
cargo new my-custom-ferron
cd my-custom-ferron
```

### 2. Configure `Cargo.toml`

Add `ferron-entrypoint` and your custom modules to the dependencies. You can choose which default features to include by toggling `profile-default`.

```toml
[package]
name = "my-custom-ferron"
version = "0.1.0" # Arbitrary version
edition = "2021"

[dependencies]
# Include the entrypoint. profile-default includes all standard modules.
# Use default-features = false if you want to select modules manually.
ferron-entrypoint = { git = "https://github.com/ferronweb/ferron.git", branch = "3.x", features = ["profile-default"] }

# Add your custom Ferron module
ferron-http-custom = { git = "https://git.example.com/ferron-http-custom.git" }
```

### 3. Implement `main.rs`

Your `main` function should initialize the entrypoint, obtain a profile (a list of module loaders), add your custom module loader to it, and then start the server.

```rust
fn main() {
    // Initialize global allocators and panic hooks
    ferron_entrypoint::init();

    // Start with the default set of modules as a base
    let mut profile = ferron_entrypoint::default_profile();

    // Register your custom module loader
    // Assumes your module provides a 'CustomModuleLoader' struct
    profile.push(Box::new(ferron_http_custom::CustomModuleLoader));

    // Transfer control to the Ferron entrypoint
    ferron_entrypoint::main(profile);
}
```

## Building and running

Build your custom binary using Cargo:

```bash
cargo build --release
```

Run your custom server with a configuration file:

```bash
./target/release/my-custom-ferron run -c ferron.conf
```

## How it works

Ferron uses a **module profile** system. The `ferron-entrypoint` crate provides the CLI logic and runtime management, but it doesn't know about specific modules until they are registered in the `Vec<Box<dyn ModuleLoader>>` passed to its `main` function.

- `ferron_entrypoint::init()`: Sets up `malloc-best-effort` and crash reporting.
- `ferron_entrypoint::default_profile()`: Returns a list of all loaders for modules bundled with Ferron.
- `ferron_entrypoint::main(profile)`: Parses command-line arguments (like `run`, `validate`, `adapt`), loads the configuration, and starts the lifecycle for all modules in the profile.

## Notes and troubleshooting

- **Dependency versions** - ensure your custom modules are compatible with the version of `ferron-core` and `ferron-entrypoint` you are using.
- **Feature flags** - if you want a minimal binary, disable `profile-default` for `ferron-entrypoint` and add only the specific modules you need to your `Cargo.toml`.
- **Static linking** - Ferron modules are statically linked. Any change to your module list requires a recompilation of the binary.
