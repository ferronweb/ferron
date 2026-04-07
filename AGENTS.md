# Ferron "Titanium" - Web Server

## Project Overview

Ferron "Titanium" is a high-performance, modular web server written in Rust. It features a plugin-based architecture that supports HTTP serving, reverse proxying, static file serving, automatic TLS via ACME, rate limiting, URL rewriting, and comprehensive observability (logging, metrics, OTLP export).

The project is organized as a Rust workspace with the following key components:

- **`bin/`** - The `ferron` CLI and service entrypoints (daemon mode, Windows service support).
- **`core/`** - Shared runtime, module registry, logging, shutdown handling, and configuration infrastructure.
- **`modules/`** - Pluggable feature crates including:
  - HTTP: `http-server`, `http-static`, `http-proxy`, `http-headers`, `http-ratelimit`, `http-response`, `http-rewrite`
  - Config: `config-json`, `config-ferronconf`
  - TLS: `tls-manual`, `tls-acme`, `ocsp-stapler`
  - Observability: `observability-consolelog`, `observability-logfile`, `observability-otlp`, `observability-format-json`, `observability-format-text`, `observability-process-metrics`
  - Admin: `admin-api`
- **`types/`** - Shared domain types for HTTP, TLS, DNS, OCSP, observability, and admin.
- **`docs/`** - Project documentation and configuration reference. Styled after `ferron/docs` (sentence-case headers, YAML frontmatter, user-facing tone, `**Configuration example:**` blocks, `## Notes and troubleshooting` sections). Navigation structure in `docs/docLinks.ts`.

## Building and Running

Run commands from the repository root.

### Build & Test
```bash
cargo build --workspace              # Build all crates
cargo run -p ferron -- --help        # Inspect CLI commands and flags
cargo test --workspace               # Run unit tests across workspace crates
cargo test -p ferron-http-server     # Run tests for a specific module
cargo bench -p ferron-http-server --features bench  # Run HTTP resolver benchmarks
```

### Linting & Formatting
```bash
cargo fmt --all --check              # Verify formatting
cargo clippy --workspace --all-targets -- -D warnings  # Fail on lint warnings
```

### Running the Server
```bash
cargo run -p ferron -- run -c ferron.conf          # Run with config file
cargo run -p ferron -- validate -c ferron.conf     # Validate configuration
cargo run -p ferron -- adapt -c ferron.conf        # Output config as JSON
```

### Daemon Mode (Unix)
```bash
cargo run -p ferron -- daemon -c ferron.conf --pid-file /var/run/ferron.pid
```

## Configuration

Ferron uses a flexible configuration system with multiple adapters:

- **`.conf` files** - Parsed by `config-ferronconf` (custom syntax, see [docs/configuration/](docs/configuration/))
- **`.json` files** - Parsed by `config-json`

Configuration is loaded from `./ferron.conf` by default. Use `--config` / `-c` to specify a different path, and `--config-adapter` to force a specific adapter. The `--verbose` flag enables debug-level logging.

See [docs/configuration/index.md](docs/configuration/index.md) for the full configuration reference.

> **IMPORTANT:** Before implementing any feature that introduces or modifies configuration directives, **read the relevant documentation in `docs/configuration/` first.** This prevents introducing invalid or inconsistent configuration syntax, directive naming, or scoping. The configuration reference defines the accepted directive names, scopes (global, admin, HTTP host), and syntax conventions that all implementations must follow.

## Development Conventions

- **Rust 2021 edition** with `rustfmt`-clean code.
- 4-space indentation, `snake_case` for modules/functions/files, `PascalCase` for structs/enums/traits.
- Extension-point naming: `*ModuleLoader`, `*Configuration*`, `*Provider*`.
- **Conventional Commits**: `feat:`, `fix:`, `refactor:`, `chore:`.
- Tests are inline `#[cfg(test)]` modules, primarily in `core/`, `bin/`, and `modules/http-server/`.
- Benchmarks live in `modules/http-server/benches/`.
- Parser, registry, runtime, and TLS changes should always include tests.
- **Documentation is written after implementation is complete.** Implement the feature first, then update the relevant `docs/configuration/` pages to reflect the final behavior and syntax.

### Documentation style

- **Sentence case** for all headers (only first word, proper nouns, acronyms, and directive names capitalized).
- **YAML frontmatter** on every page (`title` and `description`).
- **User-facing tone** — second-person ("you", "your"), approachable intros.
- **`ferron` code blocks** for all configuration examples (````ferron`).
- **Directive tables** for directive/subdirective definitions (Arguments | Description | Default).
- **H3 category groupings** — related directives grouped under semantic H3 headings.
- **`**Configuration example:**`** block after each directive group, showing complete working examples.
- **`## Notes and troubleshooting`** section at the end of every page.
- **Cross-references** use relative `./file.md` paths within configuration directory.
- `docs/docLinks.ts` defines the sidebar navigation structure.
- Directive descriptions follow the pattern: "This directive specifies [description]. Default: `value`".

## Architecture Highlights

- **Module-based architecture**: Features are loaded as pluggable modules at runtime.
- **DAG-based registry**: Stages and providers are ordered via dependency graphs.
- **Hot-reload support**: Configuration changes trigger graceful reload without full restart.
- **Cross-platform**: Supports Unix (daemon mode with PID file, signal handling) and Windows (native service management).
- **Custom allocator**: Uses `malloc-best-effort` (`BEMalloc`) as the global allocator.

## Notes

- The `ferron/` subdirectory is a separate nested git repository and is **not** part of this workspace.
