---
title: "Building Ferron 3 from source (custom modules)"
description: "How to build a custom Ferron 3 binary with external or community-developed modules."
---

The default `ferron` binary includes a broad set of modules. You can build your own custom version of Ferron to include external modules or community-developed extensions. You can also exclude default features for a smaller binary footprint.

## Prerequisites

- **Rust toolchain**: Install from [rustup.rs](https://rustup.rs/).
- **Access to Ferron source**: You will need the Ferron repository or reference `ferron-entrypoint` via git/path.

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
ferron-entrypoint = { git = "https://github.com/ferronweb/ferron.git", branch = "3.x", features = ["profile-default"] }

# Add your custom Ferron module
ferron-http-custom = { git = "https://git.example.com/ferron-http-custom.git" }
```

### 3. Implement `main.rs`

Your `main` function should initialize the entrypoint and get a profile (a list of module loaders). Add your custom module loader to the profile, then start the server.

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
> If you want a minimal binary, disable `profile-default` for `ferron-entrypoint` and add only the specific modules you need to your `Cargo.toml`.

> [!note]
> Make sure your custom modules work with the version of `ferron-core` and `ferron-entrypoint` you use. Ferron statically links modules. Any change to your module list requires you to recompile the binary.

## Building with FIPS-certified cryptography

To build a custom binary that uses only FIPS-approved cryptography, enable the `fips` feature on `ferron-entrypoint`:

```toml
[dependencies]
ferron-entrypoint = { git = "https://github.com/ferronweb/ferron.git", branch = "3.x", features = ["profile-default", "fips"] }
```

A FIPS build restricts cryptography to FIPS-approved algorithms: OCSP stapling, TLS cipher suites and key exchange groups are filtered, and HTTP basic auth password verification accepts only PBKDF2 hashes (Argon2 and scrypt are rejected). The `ferron version` command on a FIPS binary prints `This build is configured to use FIPS-certified cryptography.`

> [!note]
> Use the `fips` feature when you must run Ferron in a FIPS-compliant environment. The default build uses a broader set of algorithms and is not FIPS-certified.

## Building and running

Build your custom binary using Cargo:

```bash
cargo build --release
```

Run your custom server with a configuration file:

```bash
./target/release/my-custom-ferron run -c ferron.conf
```

## Packaging a custom build

You can package a custom Ferron binary in the same way as the default
build. The `packaging/` scripts and `just` commands expect compiled binaries
in `target/`. Because custom binaries live outside the main repository, you
copy the built `target/` directory into a clone of the stock Ferron source
and run the packaging commands there.

### 1. Build the custom binary

In your custom binary project, build a release binary:

```bash
cargo build --release
```

This creates `target/release/my-custom-ferron` (or
`target/x86_64-unknown-linux-gnu/release/my-custom-ferron` when you build with
`--target`).

### 2. Clone the stock Ferron source

```bash
git clone https://github.com/ferronweb/ferron -b 3.x ferron-stock
cd ferron-stock
```

Keep this clone clean. It provides the packaging scripts (`packaging/`,
`Justfile`, `cross-build/`) and the default configuration files (`configs/`,
`wwwroot/`).

### 3. Copy the `target` directory

Copy the entire `target` directory from your custom project into the stock
clone. The packaging scripts look for binaries in `target/release` or
`target/<triple>/release`.

```bash
# From the stock clone directory
rm -rf target
cp -r /path/to/my-custom-ferron/target target
```

> [!note]
> The packaging scripts pick up every file in `target/release` that has no
> extension (or `*.exe`, `*.so`, `*.dll`, `*.dylib` on Windows). Make sure the
> custom binary name does not clash with build artifacts you do not want to
> ship. The `packaging/archive/package.sh` script copies the binary plus
> `configs/ferron.release.conf` and `wwwroot/`.

### 4. Run packaging commands via `just`

Install `just` from `https://just.systems/` and run the packaging command
you need. Examples:

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

The output appears in `dist/` in the stock clone directory, for example
`dist/ferron-3.0.0-beta.11-x86_64-unknown-linux-musl.tar.gz`.

> [!tip]
> For cross-compilation, use `just cross-build <target> true` in the
> custom project first, then copy `target/` and run `just package <target>`.

### 5. Verify the package

Check that the archive contains your custom binary under the expected name
and that `ferron version` or `ferron directives` reports your module
directives.

## How it works

Ferron uses a module profile system. The `ferron-entrypoint` crate contains the CLI logic and runtime management. It does not know the specific modules until you register them in the `Vec<Box<dyn ModuleLoader>>` passed to its `main` function.

- `ferron_entrypoint::init()`: Sets up `malloc-best-effort` and crash reporting.
- `ferron_entrypoint::default_profile()`: Returns a list of all loaders for modules bundled with Ferron.
- `ferron_entrypoint::main(profile)`: Parses command-line arguments (like `run`, `validate`, `adapt`), loads the configuration, and starts the lifecycle for all modules in the profile.
