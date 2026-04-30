# Repository Guidelines

## Project structure & module organization
Ferron 3 is a Rust workspace. Core runtime code lives in `core/`, CLI and service entrypoints are in `bin/`, feature crates are under `modules/*`, and shared domain types are in `types/*`. End-to-end fixtures and images live in `e2e/`. User-facing documentation is in `docs/`, with sidebar structure defined in `docs/docLinks.ts`. Keep static assets near the crate that serves them, such as `modules/http-static/assets/`.

## Build, test, and development commands
Run commands from the repository root.

- `cargo build --workspace` builds every crate.
- `cargo test --workspace` runs the full test suite.
- `cargo test -p ferron-http-server` runs one crate’s tests.
- `cargo run -p ferron -- --help` lists CLI commands.
- `cargo run -p ferron -- run -c ferron.conf` starts a local server.
- `cargo fmt --all --check` verifies formatting.
- `cargo clippy --workspace --all-targets -- -D warnings` enforces lint cleanliness.

## Coding style & naming conventions
Use Rust 2021 idioms, `rustfmt` formatting, and 4-space indentation. Prefer `snake_case` for files, modules, and functions, and `PascalCase` for types and traits. Follow existing extension-point names such as `*ModuleLoader`, `*Configuration*`, and `*Provider*`. Keep modules focused and aligned with the workspace layout.

## Testing guidelines
Place tests close to the code, usually in inline `#[cfg(test)]` modules. Add tests for parser, registry, runtime, TLS, and configuration changes. Avoid trivial delegation tests, standard library behavior tests, duplicated internal coverage, and sleep-based concurrency tests.

## Commit & pull request guidelines
Use Conventional Commits such as `feat:`, `fix:`, `docs:`, `test:`, `refactor:`, and `chore:`. Keep subjects short and imperative. Pull requests should explain the change, list validation performed, and link related issues when relevant. Include screenshots only for UI or documentation work that needs visual confirmation.

## Security, configuration, and docs
If you change configuration directives or syntax, read and update the matching pages under `docs/configuration/`. Validate config changes with `cargo run -p ferron -- validate -c ferron.conf`. Documentation should use sentence-case headings, YAML frontmatter, `ferron` code blocks, relative links, and a `## Notes and troubleshooting` section.
