---
title: "Building Ferron 3 from source"
description: "How to build Ferron 3 from source using Cargo."
---

This page describes how to build Ferron 3 from source. This is currently the only supported installation method.

## Prerequisites

Before building Ferron, make sure you have the following installed:

- **Rust toolchain** — Ferron is written in Rust and requires `cargo` to build. You can install Rust from [rustup.rs](https://rustup.rs/).
- **Git** — needed to clone the repository.

## Building from source

Clone the Ferron repository and check out the latest development branch:

```sh
git clone https://github.com/ferronweb/ferron -b develop-3.x
cd ferron
```

Build the entire workspace:

```sh
cargo build -r --workspace
```

This compiles all crates in the workspace, including the `ferron` binary and all module crates.

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

Ferron provides several commands for working with configuration files:

```sh
cargo run -r -p ferron -- validate -c ferron.conf   # validate configuration without starting
cargo run -r -p ferron -- adapt -c ferron.conf      # output configuration as JSON
```

### Running as a daemon (Unix)

On Unix systems, you can run Ferron as a background daemon with a PID file:

```sh
cargo run -r -p ferron -- daemon -c ferron.conf --pid-file /var/run/ferron.pid
```

## Running tests and checks

Before submitting changes or if you suspect issues, run the full test suite and code checks:

```sh
cargo test --workspace                              # run all workspace tests
cargo fmt --all --check                             # verify code formatting
cargo clippy --workspace --all-targets -- -D warnings  # run linter with warnings as errors
```

## Notes and troubleshooting

- **Build times** — the first build will take longer as Cargo downloads and compiles all dependencies. Subsequent builds are faster.
- **All modules are compiled into the binary** — Ferron uses a module-driven architecture where all modules are statically linked. No runtime plugin loading is available yet.
- **Primary testing target is Linux** — Windows and macOS receive less coverage. If you encounter platform-specific issues, please report them.
- **Alpha quality** — Ferron 3 is an early development release. APIs and configuration may change.
