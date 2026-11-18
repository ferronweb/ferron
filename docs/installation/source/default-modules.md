---
title: "Building Ferron 3 from source (default modules)"
description: "How to build Ferron 3 from source using Cargo (with default modules)."
---

Build Ferron 3 from source with the default module set.

## Prerequisites

- Rust toolchain with `cargo`. Install it from [rustup.rs](https://rustup.rs/).
- Git, to clone the repository.

## Building from source

Clone the Ferron repository and check out the `3.x` branch:

```sh
git clone https://github.com/ferronweb/ferron -b 3.x
cd ferron
```

Build the whole workspace:

```sh
cargo build -r --workspace
```

The build compiles every crate in the workspace, including the `ferron` binary and all module crates.

## Building with FIPS-certified cryptography

To compile Ferron with only FIPS-approved cryptographic algorithms, enable the `fips` feature for the `ferron` binary:

```sh
cargo build -r -p ferron --features=fips
```

A FIPS build restricts the cryptography that Ferron uses:

- OCSP stapling disables SHA1, Ed25519, and secp256k1 algorithms.
- TLS cipher suites and key exchange groups are filtered to FIPS-approved algorithms.
- HTTP basic auth password verification accepts only PBKDF2 hashes. Argon2, bcrypt, and scrypt hashes are rejected because those algorithms are not FIPS-approved.

> [!note]
> Use a FIPS build when you must run Ferron in a FIPS-compliant environment. The default build supports more algorithms and is not FIPS-certified.

Run `ferron version` to confirm a FIPS build. It prints `This build is configured to use FIPS-certified cryptography.`.

> [!note]
> The first build takes longer because Cargo downloads and compiles all dependencies. Later builds are faster.

## Running the server

Run Ferron through Cargo after the build:

```sh
cargo run -r -p ferron -- run -c ferron.conf
```

For debug-level logging:

```sh
cargo run -r -p ferron -- run -c ferron.conf --verbose
```

### Other CLI commands

```sh
cargo run -r -p ferron -- validate -c ferron.conf   # validate configuration without starting
cargo run -r -p ferron -- adapt -c ferron.conf      # output configuration as JSON
```

### Run as a daemon (Unix)

Run Ferron as a background daemon with a PID file:

```sh
cargo run -r -p ferron -- daemon -c ferron.conf --pid-file /var/run/ferron.pid
```

Reload the daemon through its PID file:

```sh
kill -HUP $(cat /var/run/ferron.pid)
```

## Running tests and checks

Run the full test suite and code checks before you submit changes or when you suspect an issue:

```sh
cargo test --workspace                              # run all workspace tests
cargo fmt --all --check                             # verify code formatting
cargo clippy --workspace --all-targets -- -D warnings  # run linter with warnings as errors
```
