---
title: "Building Ferron 3 from source (default modules)"
description: "How to build Ferron 3 from source using Cargo (with default modules)."
---

This page describes how to build Ferron 3 from source, with default modules.

## Prerequisites

Before building Ferron, make sure you have the following installed:

- **Rust toolchain**: Ferron uses the Rust language and requires `cargo` to build. You can install Rust from [rustup.rs](https://rustup.rs/).
- **Git**: use it to clone the repository.

## Building from source

Clone the Ferron repository and check out the latest development branch:

```sh
git clone https://github.com/ferronweb/ferron -b 3.x
cd ferron
```

Build the entire workspace:

```sh
cargo build -r --workspace
```

This compiles all crates in the workspace, including the `ferron` binary and all module crates.

## Building with FIPS-certified cryptography

You can compile Ferron to use only FIPS-approved cryptographic algorithms. Enable the `fips` feature when you build the `ferron` binary:

```sh
cargo build -r -p ferron --features=fips
```

A FIPS build restricts the cryptography that Ferron uses:

- TLS cipher suites and key exchange groups are filtered to FIPS-approved algorithms.
- HTTP basic auth password verification accepts only PBKDF2 password hashes. Argon2 and scrypt hashes are rejected, because those algorithms are not FIPS-approved.

> [!note]
> Use a FIPS build when you need to run Ferron in a FIPS-compliant environment. The default build uses a broader set of algorithms and is not FIPS-certified.

To confirm that your binary is a FIPS build, run `ferron version`. It prints `This build is configured to use FIPS-certified cryptography.`.

> [!note]
> The first build will take longer as Cargo downloads and compiles all dependencies. Later builds are faster.

## Running the server

Once the build completes, you can run Ferron directly with `cargo run`:

```sh
cargo run -r -p ferron -- run -c ferron.conf
```

To enable debug-level logging, add the `--verbose` flag:

```sh
cargo run -r -p ferron -- run -c ferron.conf --verbose
```

### Other CLI commands

Ferron has several commands for working with configuration files:

```sh
cargo run -r -p ferron -- validate -c ferron.conf   # validate configuration without starting
cargo run -r -p ferron -- adapt -c ferron.conf      # output configuration as JSON
```

### Running as a daemon (Unix)

On Unix systems, you can run Ferron as a background daemon with a PID file:

```sh
cargo run -r -p ferron -- daemon -c ferron.conf --pid-file /var/run/ferron.pid
```

You can then reload the daemon using its PID file:

```sh
kill -HUP $(cat /var/run/ferron.pid)
```

## Running tests and checks

Before submitting changes or if you suspect issues, run the full test suite and code checks:

```sh
cargo test --workspace                              # run all workspace tests
cargo fmt --all --check                             # verify code formatting
cargo clippy --workspace --all-targets -- -D warnings  # run linter with warnings as errors
```
