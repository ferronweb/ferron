---
title: "Example modules"
description: "Runnable Ferron 3 module examples you can copy, with links to the example repository."
---

The Ferron team maintains a repository of runnable example modules. Each crate
is small (~150 lines), has inline comments, and covers one extension point.

## Repository

- **GitHub**: `https://github.com/ferronweb/ferron3-example-modules`
- **Branch**: `3.x` (upstream Ferron is fetched from
  `https://github.com/ferronweb/ferron` branch `3.x`)
- **Workspace**: `members = ["modules/*"]`, `exclude = ["e2e", "fixture"]`

Clone and build:

```bash
git clone https://github.com/ferronweb/ferron3-example-modules
cd ferron3-example-modules
cargo build --workspace
cargo test --workspace
```

## What each example shows

| Crate | Extension point | Config |
| --- | --- | --- |
| `ferron-http-header-append` | `Stage<HttpContext>` | `example_header <value>` |
| `ferron-http-hello` | `Stage<HttpContext>` short-circuit | `hello_path`, `hello_message` |
| `ferron-echo-server` | `Module` + `Runtime` | `echo_server { listen <addr> }` |
| `ferron-observability-memory` | `Provider<ObservabilityContext>` | `observability { provider memory }` |
| `ferron-tls-selfsigned` | `Provider<TlsContext>` | `tls { provider selfsigned }` |
| `ferron-dns-memory` | `Provider<DnsContext>` | `dns { provider memory }` |
| `ferron-config-toml` | `ConfigurationAdapter` | `--config-adapter toml` |
| `ferron-logformat-csv` | `Provider<LogFormatterContext>` | `format csv` |

Each `src/lib.rs` starts with a doc comment that shows the `ferron` config
block, explains the flow, and points to the relevant Ferron source file.

## Fixture binary

`fixture/` builds a Ferron binary that wires all examples plus the default
profile:

```bash
cargo run -p ferron-example-fixture -- run -c configs/ferron.conf
cargo run -p ferron-example-fixture -- validate -c configs/ferron.conf
cargo run -p ferron-example-fixture -- directives | jq .
```

The fixture `Cargo.toml` depends on `ferron-entrypoint` via git:

```toml
ferron-entrypoint = { git = "https://github.com/ferronweb/ferron", branch = "3.x", features = ["profile-default"] }
```

When the example repository lives inside the upstream checkout for local
development, the parent `Cargo.toml` patches git dependencies to the local path
so builds work offline.

## E2E tests

`e2e/` runs real HTTP requests against Ferron in Docker via `testcontainers`:

```bash
docker build -f e2e/Dockerfile.test -t e2e-test-ferron-example:latest .
cd e2e && cargo test
```

See `e2e/Dockerfile.test` and `e2e/tests/` for the pattern. The tests are
`#[ignore]`-ready placeholders you can flesh out for your own modules.

## Using an example as a starting point

1. Copy the crate directory (`cp -r modules/http-hello my-module`).
2. Rename the package and `name()` strings.
3. Adjust the directive and validator.
4. Add it to your custom binary (see
   [Creating a module](/docs/v3/module-development/creating-a-module)).
5. Run `ferron validate`, `ferron directives`, and unit tests.

> [!tip]
> Start with `ferron-http-header-append` or `ferron-http-hello`. They are the
> shortest and cover validation, directives, and stage ordering.
