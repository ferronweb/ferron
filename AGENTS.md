# Repository guidelines

## Project structure & module organization
Ferron is a Rust workspace with multiple crates. Core server code lives in `ferron/` (`src/main.rs`, runtime/setup/config modules). Shared interfaces and utilities are in `ferron-common/`. Built-in feature crates include `ferron-modules-builtin/`, `ferron-observability-builtin/`, and `ferron-dns-builtin/`. Utility binaries are in `ferron-passwd/`, `ferron-precompress/`, and `ferron-yaml2kdl/`.

Tests are split by scope:
- Workspace/unit/integration tests via Cargo in each crate.
- Smoke tests in `smoketest/`.
- Docker-based end-to-end tests in `e2e/tests/`.

Docs live in `docs/`; packaging assets are under `packaging/`, `installer/`, and `configs/`.

## Build, test, and development commands
Run from repo root unless noted.

- `make build-dev` builds Ferron in debug mode through the project workflow (`build/prepare` + generated workspace).
- `make run-dev` builds and runs `target/debug/ferron` (auto-copies `configs/ferron.test.kdl` to `ferron.kdl` if missing).
- `cargo test --workspace --verbose` runs Rust tests across the workspace.
- `cargo fmt --all -- --check` verifies formatting.
- `cargo clippy --workspace -- -D warnings` treats warnings as errors.
- `make smoketest-dev` runs smoke tests against the debug binary.
- `cd e2e && cargo test` runs Docker/Testcontainers E2E tests.

## Coding style & naming conventions
Formatting is enforced by `rustfmt.toml`: 2-space indentation, max width 120, import reordering enabled. Use `snake_case` for functions/modules/files, `PascalCase` for types/traits, and descriptive module names matching domain areas (`config`, `listeners`, `optional`, etc.).

## Testing guidelines
Prefer targeted tests near changed code plus `cargo test --workspace`. If behavior affects networking, config parsing, modules, or containers, also run smoke tests and relevant E2E tests. E2E suites are file-based (`e2e/tests/<feature>.rs`) and should be registered in `e2e/Cargo.toml`.

## Commit & pull request guidelines
Use focused commits with conventional prefixes seen in history (for example: `fix: ...`, `refactor: ...`, `chore: ...`, `ci: ...`). Open PRs against `develop-2.x`. Include:
- Clear problem/solution summary.
- Validation commands run (exact commands).
- Linked issues when applicable.
- Matching docs updates in `docs/` (and `docs/docLinks.ts` if pages were added/renamed).
