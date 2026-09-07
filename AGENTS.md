# Repository guidelines for AI agents

Human-readable source of truth lives in:

- [`CONTRIBUTING.md`](./CONTRIBUTING.md): repository layout, build and test commands, testing tiers (unit, E2E, fuzz, benchmarks), dual runtime (`zincio` primary and `tokio` secondary), `TODO`/`FIXME` conventions, mandatory `feat:`/`fix:` updates (docs, changelog, E2E tests), changelog format, cross-compilation, Docker builds.
- [`docs/README.md`](./docs/README.md): docs style guide, idiomatic Ferron 3 configuration style, invalid-example marker (`# INVALID`), docs validation.

Follow those pages. Do not duplicate their rules here to avoid drift.

## Quick reference

- Branch from and target `develop-3.x`.
- Use Conventional Commits (`feat:`, `fix:`, `docs:`, `test:`, `refactor:`, `chore:`).
- Every `feat:` or `fix:` must update docs, `CHANGELOG.md` (if user-facing), and E2E tests (if applicable). `docs:` commits may touch docs alone.
- Validate config docs with `cargo run -p ferron -- validate -c ferron.conf` and `cargo run --manifest-path doctest/Cargo.toml`.
- Check formatting and lints with `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings`.
