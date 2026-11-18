---
title: "Building Ferron 3 from source (custom modules)"
description: "How to build a custom Ferron 3 binary with external or community-developed modules."
---

The default `ferron` binary includes a broad set of modules. You can build a custom binary with external modules or community extensions. You can also drop default features for a smaller binary.

## Prerequisites

- Rust toolchain from [rustup.rs](https://rustup.rs/).
- Access to the Ferron source. Either clone the repository, or reference `ferron-entrypoint` by git or path.

## Creating a custom binary project

Create a new Rust binary crate that depends on `ferron-entrypoint`.

### 1. Initialize the project

```bash
cargo new my-custom-ferron
cd my-custom-ferron
```

### 2. Configure `Cargo.toml`

Add `ferron-entrypoint` and the custom modules to the dependencies. The `profile-default` feature selects the default module set.

```toml
[package]
name = "my-custom-ferron"
version = "0.1.0" # Arbitrary version
edition = "2021"

[dependencies]
# Include the entrypoint. profile-default includes all standard modules.
ferron-entrypoint = { git = "https://github.com/ferronweb/ferron.git", branch = "3.x", features = ["profile-default"] }

# Add your custom Ferron module
ferron-http-custom = { git = "https://git.example.com/ferron-http-custom.git" }
```

### 3. Implement `main.rs`

The `main` function initializes the entrypoint, gets a profile (a list of module loaders), adds the custom loader, and starts the server.

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

> [!tip]
> For a minimal binary, disable `profile-default` for `ferron-entrypoint` and add only the modules you need to `Cargo.toml`.

> [!note]
> Keep the custom modules compatible with the `ferron-core` and `ferron-entrypoint` versions that you use. Ferron links modules statically, so any change to the module list needs a recompile.

## Building with FIPS-certified cryptography

To build a custom binary with only FIPS-approved cryptography, enable the `fips` feature on `ferron-entrypoint`:

```toml
[dependencies]
ferron-entrypoint = { git = "https://github.com/ferronweb/ferron.git", branch = "3.x", features = ["profile-default", "fips"] }
```

A FIPS build restricts cryptography to FIPS-approved algorithms:

- OCSP stapling, TLS cipher suites, and key exchange groups are filtered.
- HTTP basic auth password verification accepts only PBKDF2 hashes. Argon2, scrypt, and bcrypt are rejected.

`ferron version` on a FIPS binary prints `This build is configured to use FIPS-certified cryptography.`.

> [!note]
> Use the `fips` feature when you must run Ferron in a FIPS-compliant environment. The default build supports more algorithms and is not FIPS-certified.

## Building and running

```bash
cargo build --release
```

Run the custom server with a configuration file:

```bash
./target/release/my-custom-ferron run -c ferron.conf
```

## Packaging a custom build

You can package a custom Ferron binary like the default build. The `packaging/` scripts and `just` commands expect compiled binaries in `target/`. Because the custom binary lives outside the main repository, copy its `target/` directory into a clone of the stock Ferron source and run the packaging commands there.

### 1. Build the custom binary

```bash
cargo build --release
```

This creates `target/release/my-custom-ferron`, or `target/x86_64-unknown-linux-gnu/release/my-custom-ferron` when you build with `--target`.

### 2. Clone the stock Ferron source

```bash
git clone https://github.com/ferronweb/ferron -b 3.x ferron-stock
cd ferron-stock
```

Keep this clone clean. It provides the packaging scripts (`packaging/`, `Justfile`, `cross-build/`) and the default configuration files (`configs/`, `wwwroot/`).

### 3. Copy the `target` directory

Copy the whole `target` directory from the custom project into the stock clone. The packaging scripts look for binaries in `target/release` or `target/<triple>/release`.

```bash
# From the stock clone directory
rm -rf target
cp -r /path/to/my-custom-ferron/target target
```

> [!note]
> The packaging scripts pick up every file in `target/release` that has no extension, or that has an `.exe`, `.so`, `.dll`, or `.dylib` extension on Windows. Make sure the custom binary name does not clash with build artifacts that you do not want to ship. `packaging/archive/package.sh` copies the binary plus `configs/ferron.release.conf` and `wwwroot/`.

### 4. Run packaging commands with `just`

Install `just` from [just.systems](https://just.systems/) and run the command you need:

```bash
just package                           # archive for host triple (tar.gz or zip)
just package x86_64-unknown-linux-musl # archive for explicit target
just package-deb x86_64-unknown-linux-musl  # Debian package (uses Docker)
just package-rpm x86_64-unknown-linux-musl  # RPM package (uses Docker)

# Windows (run in PowerShell)
just package                           # zip for host
just package-windows x86_64-pc-windows-msvc
```

For FIPS builds, pass the FIPS flag:

```bash
just package "" true                   # FIPS archive for host
just package x86_64-unknown-linux-musl true
just package-deb x86_64-unknown-linux-musl true
```

The output appears in `dist/` in the stock clone directory, for example `dist/ferron-3.0.0-beta.11-x86_64-unknown-linux-musl.tar.gz`.

> [!tip]
> For cross-compilation, run `just cross-build <target> true` in the custom project first, then copy `target/` and run `just package <target>`.

### 5. Verify the package

Check that the archive contains the custom binary under the expected name. Check that `ferron version` or `ferron directives` reports the module directives.

## How it works

Ferron uses a module profile system. The `ferron-entrypoint` crate contains the CLI logic and runtime management. It does not know the modules until you register them in the `Vec<Box<dyn ModuleLoader>>` passed to its `main` function.

- `ferron_entrypoint::init()`: sets up `malloc-best-effort` and crash reporting.
- `ferron_entrypoint::default_profile()`: returns the loaders for the modules bundled with Ferron.
- `ferron_entrypoint::main(profile)`: parses command-line arguments such as `run`, `validate`, and `adapt`, loads the configuration, and starts the lifecycle for every module in the profile.
