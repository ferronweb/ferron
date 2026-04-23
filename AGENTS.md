# Repository Guidelines

## Project Structure & Module Organization
Ferron 3 is a Rust workspace. Core runtime and shared infrastructure live in `core/`, the `ferron` CLI and service entrypoints are in `bin/`, feature crates are under `modules/*`, and shared domain types are under `types/*`. End-to-end test assets live in `e2e/`, and user-facing documentation is in `docs/`. Static assets are kept close to the crates that serve them, such as `modules/http-static/assets/` and `bin/assets/`.

## Build, Test, and Development Commands
Run commands from the repository root.

- `cargo build --workspace` builds all workspace crates.
- `cargo test --workspace` runs the full test suite.
- `cargo test -p ferron-http-server` runs tests for one crate or module.
- `cargo run -p ferron -- --help` inspects CLI commands.
- `cargo run -p ferron -- run -c ferron.conf` starts the server with a config file.
- `cargo fmt --all --check` verifies formatting.
- `cargo clippy --workspace --all-targets -- -D warnings` enforces lint cleanliness.

## Coding Style & Naming Conventions
Use Rust 2021 conventions, `rustfmt`-clean code, and 4-space indentation. Keep names `snake_case` for files, modules, and functions, and `PascalCase` for types and traits. Follow the existing extension-point naming patterns such as `*ModuleLoader`, `*Configuration*`, and `*Provider*`. Prefer small, focused modules that match the workspace layout.

## Testing Guidelines
Tests are usually inline `#[cfg(test)]` modules, especially in `core/`, `bin/`, and `modules/http-server/`. Add tests for parser, registry, runtime, TLS, and config changes. Avoid trivial delegation tests, stdlib behavior tests, and sleep-based timing tests. Keep unit tests close to the component they cover.

## Commit & Pull Request Guidelines
Recent history uses Conventional Commits: `feat:`, `fix:`, `refactor:`, `perf:`, `docs:`, `test:`, and `chore:`. Keep commit subjects short and imperative. Pull requests should explain the change, list validation performed, and link related issues when relevant. Include screenshots only for UI or docs changes that need visual confirmation.

## Security & Configuration Tips
If you change configuration directives or syntax, read the relevant pages in `docs/configuration/` first and update documentation after the implementation is complete. Validate config-related work with `cargo run -p ferron -- validate -c ferron.conf` before merging.

## Unneeded and redundant tests

To maintain a clean and efficient test suite, avoid adding or maintaining tests in the following categories:
- **Trivial Delegation Tests:** Avoid tests that merely verify that a wrapper method correctly delegates to an underlying library (e.g., `HttpContext` methods wrapping `typemap-rev`). These should be covered by integration tests rather than repetitive unit tests.
- **Internal Component Duplication:** Keep unit tests close to the components they test (e.g., in `stage2.rs` for radix tree logic). Avoid duplicating detailed internal tests in high-level integration files like `resolver.rs`.
- **Trivial Property Tests:** Do not add tests for fundamental language features or trivial struct initialization (e.g., "roundtrip" tests that only verify field assignment).
- **Inefficient Concurrent Tests:** Avoid tests that use `thread::sleep` for timing. Use proper synchronization or mock clocks if timing is necessary.
- **Standard Library Behavior:** Do not test the parsing or error-handling logic of the Rust standard library (e.g., bare `IpAddr` or `SocketAddr` parsing).

## Documentation style
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
