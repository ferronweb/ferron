<p align="center">
  <a href="https://ferron.sh" target="_blank">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="wwwroot/assets/logo-dark.png">
      <img alt="Ferron logo" src="wwwroot/assets/logo.png" width="256">
    </picture>
  </a>
</p>
<p align="center">
  <b>Ferron</b> - a fast, modern, and easily configurable web server with automatic TLS
</p>

* * *

<p align="center">
  <a href="https://ferron.sh/docs" target="_blank"><img alt="Static Badge" src="https://img.shields.io/badge/Documentation-orange?style=for-the-badge"></a>
  <a href="https://ferron.sh" target="_blank"><img alt="Website" src="https://img.shields.io/website?url=https%3A%2F%2Fferron.sh&style=for-the-badge"></a>
  <a href="https://matrix.to/#/#ferronweb:matrix.org" target="_blank"><img alt="Chat" src="https://img.shields.io/matrix/ferronweb%3Amatrix.org?style=for-the-badge"></a>
  <a href="https://x.com/ferron_web" target="_blank"><img alt="X (formerly Twitter) Follow" src="https://img.shields.io/twitter/follow/ferron_web?style=for-the-badge"></a>
  <a href="https://hub.docker.com/r/ferronserver/ferron" target="_blank"><img alt="Docker Pulls" src="https://img.shields.io/docker/pulls/ferronserver/ferron?style=for-the-badge"></a>
  <a href="https://github.com/ferronweb/ferron" target="_blank"><img alt="GitHub Repo stars" src="https://img.shields.io/github/stars/ferronweb/ferron?style=for-the-badge"></a>
</p>

> **Status: Alpha** — This is an early development release. Not production-ready. APIs and configuration may change.

## Why Ferron 3?

- **High performance** - thoroughly optimized for speed with support for high concurrency.
- **Easy configuration** - simple, intuitive configuration with sensible, secure defaults and [comprehensive documentation](https://ferron.sh/docs).
- **Modular architecture** - pipeline stages and providers registered at runtime, no recompilation needed.
- **Observability by design** - structured logs, metrics, and tracing through a unified event system.
- **Layered configuration** - composable `ferron.conf` with snippets, conditionals, and host scopes.
- **Automatic TLS** - ACME certificate acquisition and renewal with Let's Encrypt.
- **Powerful reverse proxy** - load balancing, health checks, and connection pooling.
- **Memory-safe** - built with [Rust](https://rust-lang.org/).

## What's different from Ferron 2

Ferron 3 is a complete rewrite. It shares the vision but not the code:

| Aspect | Ferron 2 | Ferron 3 |
| --- | --- | --- |
| Architecture | Monolithic | Module-driven, pluggable |
| Observability | Basic logging | Structured events with multiple backends |
| Configuration | KDL-based | Custom `.conf`, layered scopes, snippets |
| Extensibility | Compile-time modules | Runtime-registered stages and providers |
| Request Processing | Linear pipeline | DAG-ordered stages with inverse cleanup |

## Configuration examples

### Static file serving

```ferron
example.com {
    root "/var/www/html"
    directory_listing

    tls true {
        provider acme
        email "admin@example.com"
    }
}
```

### Reverse proxy

```ferron
api.example.com {
    proxy http://localhost:8080 {
        keepalive true
    }
}
```

More examples are available in the [configuration documentation](https://ferron.sh/docs/v3/configuration).

## Building from source

```sh
git clone https://github.com/ferronweb/ferron -b develop-3.x
cd ferron
cargo build --workspace
```

Run the server:

```sh
cargo run -p ferron -- run -c ferron.conf
cargo run -p ferron -- run -c ferron.conf --verbose  # with debug logging
```

Other CLI commands:

```sh
cargo run -p ferron -- validate -c ferron.conf   # validate without starting
cargo run -p ferron -- adapt -c ferron.conf      # output config as JSON
cargo run -p ferron -- daemon -c ferron.conf --pid-file /var/run/ferron.pid  # Unix daemon
```

Run tests and checks:

```sh
cargo test --workspace
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Features

What currently works in this alpha:

| Category | Modules |
| --- | --- |
| HTTP | static files, reverse proxy, forward proxy, compression, rate limiting, headers/CORS, URL rewriting, basic auth, response control |
| TLS | manual certificates, ACME automatic TLS, OCSP stapling |
| Observability | console log, file log (with rotation), OTLP export, process metrics |
| Admin | health, status, config dump, hot reload |
| Runtime | io_uring (Linux), PROXY protocol v1/v2, SIGHUP hot reload |

## Observability

Ferron 3 emits structured events through a unified pipeline. Every HTTP request generates access logs, metrics, and trace spans — all tagged with the same `trace_id` for correlated queries.

Backends: console, file (JSON or Combined Log Format), and OTLP (OpenTelemetry Protocol).

See the [observability documentation](https://ferron.sh/docs/v3/configuration/observability) for details.

## Configuration reference

The full directive reference is in [docs/configuration/](https://ferron.sh/docs/v3/configuration). Scopes:

- **Global** — `{ ... }` blocks (runtime, TCP listeners, admin API)
- **HTTP host** — `example.com { ... }` blocks (TLS, proxy, static, logging)
- **Location/conditional** — `location`, `if`, `if_not` blocks inside hosts

## Roadmap

Planned direction:

- Dynamically loadable modules (WebAssembly?)
- HTTP/3 and QUIC support
- Additional observability backends (Prometheus, Jaeger/Zipkin?)
- More authentication methods (JWT?, OAuth2?, mTLS?)
- HTTP response caching

## Known limitations

- Alpha quality — not battle-tested, expect bugs.
- Admin API has no authentication — bind it to localhost.
- All modules are compiled into the binary; no runtime plugin loading yet.
- Primary testing target is Linux. Windows and macOS receive less coverage.

## Contributing

Feedback, bug reports, and testing are welcome. When reporting issues, include your configuration file, `--verbose` output, and steps to reproduce.

## License

MIT. See `LICENSE` for details.
