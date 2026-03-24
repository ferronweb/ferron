# Ferron E2E test suite

This directory contains the end-to-end test suite for the Ferron web server, written in Rust. It uses [Testcontainers](https://testcontainers.com/) to spin up Ferron and helper services (like backends or databases) in Docker containers and verifies their behavior using standard Rust testing tools.

## Prerequisites

- **Docker** - the tests rely on Docker to run Ferron and auxiliary services. Ensure the Docker daemon is running and accessible by the user running the tests.
- **Rust** - you need a standard Rust toolchain installed (Cargo).

## How to run tests?

To run the entire suite, execute the following command in this directory:

```bash
cargo test
```

To run a specific test file (e.g., `acme`):

```bash
cargo test --test acme
```

## Rebuilding the Ferron test image

The test suite automatically builds a Docker image for Ferron (`e2e-test-ferron`) from the local source code. However, if the image is not rebuilt, the tests will use already-built images, which might not reflect the latest changes.

To force a rebuild of the web server Docker image (e.g., after modifying Ferron's source code), you need to remove the existing containers and the image. On Linux hosts, you can run:

```bash
docker rm -f $(docker ps -a --filter ancestor=e2e-test-ferron -q)
docker image rm e2e-test-ferron
```

The next time you run `cargo test`, the image will be rebuilt.

## How to write tests?

Tests are located in the `tests/` directory. Each test file is typically defined as a separate integration test in `Cargo.toml`.

To add a new test suite:

1.  Create a new Rust source file in `tests/` (e.g., `tests/my_new_feature.rs`).
2.  Register the test in `Cargo.toml`:

    ```toml
    [[test]]
    name = "my_new_feature"
    path = "tests/my_new_feature.rs"
    ```

3.  Implement your tests using `testcontainers` to spawn Ferron and `reqwest` (or other clients) to verify behavior. See existing tests like `tests/static.rs` or `tests/rproxy.rs` for examples of setting up the environment and writing configuration files dynamically.
