# Contributing to Ferron

**Thank you for helping improve Ferron!**

## Before you start

- Read the `README.md` for build and run basics.
- Follow the `CODE_OF_CONDUCT.md`.
- If your change is security-sensitive, report it privately as described in `SECURITY.md` instead of opening a public issue.

## What to contribute

Contributions are welcome across:

- bug fixes
- new features
- performance improvements
- tests
- documentation updates

## Development setup

1. Fork the repository and clone your fork.
2. Create a branch from `develop-3.x` (this is the default development branch for Ferron 3 work; CI workflows filter on it and the 3.x docs site syncs from it).
3. Make your changes in focused commits.

## Repository layout

Ferron 3 is a Rust workspace (resolver "2"). Key directories include:

- `core/`: the Ferron core
- `bin/`: thin wrapper around `ferron-entrypoint`
- `entrypoint/`: the entrypoint binary that wires all modules
- `modules/*`: where individual module crates are located
- `types/*`: shared types that can be used by modules
- `e2e/`: E2E tests via testcontainers (requires Docker and `protoc`)
- `docs/`: user-facing docs; sidebar in `docs/links.json`
- `doctest/`: setup for testing Ferron configurations in doc examples
- `utils/`: CLI utilities (`fmt`, `kdl2ferron`, `passwd`, `precompress`, `serve`)

## Build and check locally

Run from the repository root unless noted:

| Command                                                 | Purpose                                                                              |
| ------------------------------------------------------- | ------------------------------------------------------------------------------------ |
| `cargo build --workspace`                               | Build all workspace crates                                                           |
| `cargo test --workspace`                                | Run unit and inline tests                                                            |
| `cargo test -p <crate>`                                 | Run tests for a single crate                                                         |
| `cargo fmt --all --check`                               | Check formatting (the repo uses default `rustfmt` settings, no `.rustfmt.toml`)      |
| `cargo clippy --workspace --all-targets -- -D warnings` | Lint; warnings fail the check                                                        |
| `cargo shear`                                           | Check for unused dependencies (also runs in CI)                                      |
| `cargo run --manifest-path doctest/Cargo.toml`          | Test Ferron configurations in doc examples                                           |
| `cd e2e && cargo test`                                  | Run E2E tests (needs Docker and `protoc`; build the test image first, see below)     |
| `rumdl fmt docs && rumdl check --fix docs`              | Format and lint docs Markdown (requires `rumdl`)                                     |
| `npx aislop scan`                                       | Scan for possible AI-generated issues (false positives are possible; requires `npm`) |

`just` automates packaging and related tasks. Run `just --list` to see available recipes.

### Run the server from source

```bash
cargo run -p ferron -- run -c ferron.conf                         # start
cargo run -p ferron -- validate -c ferron.conf                    # validate config
cargo run -p ferron -- doctor -c ferron.conf                      # best-practice check
cargo run -p ferron -- version                                    # version and build info
```

See `docs/` for details and more commands.

## Testing structure

There are three tiers:

1. **Inline unit tests**: `#[cfg(test)] mod tests` inside source files. Run them with `cargo test --workspace` or `cargo test -p <crate>`.
2. **E2E tests**: `e2e/tests/`, each file declared as `[[test]]` in `e2e/Cargo.toml`. They use `testcontainers` and `reqwest` and need a Docker daemon and `protoc` in `PATH`. Build the test image first:

   ```bash
   docker build -f e2e/Dockerfile.test -t e2e-test-ferron:latest .
   cd e2e && cargo test
   ```

   If your change affects runtime behavior, networking, modules, container packaging, or config parsing, run the E2E suite.

3. **Fuzz**: `fuzz/` uses nightly `cargo-fuzz` and is excluded from the main workspace. Run from inside `fuzz/` (list files in `fuzz/fuzz_targets/` for available targets):

   ```bash
   cargo +nightly fuzz run $FUZZ_TARGET
   ```

   Dictionaries and seed corpora live in `fuzz/dictionaries/` and `fuzz/corpus/`.

Benchmarks live in `modules/http-server/benches/` (Criterion, gated on `features = ["bench"]` on the `ferron-http-server` crate).

## Runtime note

Ferron uses a dual runtime: primary threads run `zincio` (one per CPU, pinned, optional `io_uring`) and a secondary runtime is `tokio`. Most request handling runs on the primary runtime, so when you write HTTP server modules, prefer `zincio` functions over `tokio` equivalents.

## Code conventions

- When you leave a stub implementation, add a `TODO` marker in the comment that explains the stub.
- When you leave a comment about a known issue, add a `FIXME` marker.

## Documentation expectations

If behavior, configuration, CLI output, installation steps, or defaults change, update documentation in the same pull request.

- Main docs live in `docs/`.
- If you add or rename doc pages, update `docs/links.json`.
- If you change configuration directives, update the matching pages under `docs/configuration/` and validate with:

  ```bash
  cargo run -p ferron -- validate -c ferron.conf
  cargo run --manifest-path doctest/Cargo.toml
  ```

- Keep examples and command snippets aligned with the code and scripts in this repository.
- Config examples use `.conf` or `.ferron` file extensions. If you show an invalid configuration on purpose, prepend `# INVALID` to exactly the first line. For the full docs style guide, see [docs/README.md](./docs/README.md).

Every `feat:` or `fix:` commit must include updates to documentation (under `docs/`), the changelog (`CHANGELOG.md`, unless the change is a subtle implementation detail with no user-visible effect), and E2E tests (`e2e/tests/`, when applicable). This keeps docs from drifting and keeps changes verified. The `docs:` commit type is the exception: it may update documentation alone without code or tests.

Optional local docs linting/formatting (same tool used in CI):

```bash
rumdl fmt docs
rumdl check --fix docs
```

For other guidelines for writing documentation, see [docs/README.md](./docs/README.md).

## Cross-compilation and Docker builds

- Linux cross-builds use `cross-build/build.sh` (PGO by default) on Linux hosts and `cross` otherwise. See [cross-build/README.md](./cross-build/README.md). Non-`cross` builds require `bindgen-cli`.
- Docker images use PGO builds (`Dockerfile` distroless and musl, `Dockerfile.alpine`, `Dockerfile.debian` glibc-slim). Build a no-PGO variant with `--build-arg NOPGO=1`.

## Pull request guidelines

- Open pull requests against `develop-3.x` by default.
- Use a clear title and description explaining:
  - what changed
  - why it changed
  - how you validated it (commands you ran)
- Link related issues when applicable.
- Keep pull requests focused; separate unrelated changes.
- Ensure CI is green before requesting review.

## Commit guidance

- Commit messages follow Conventional Commits (`feat:`, `fix:`, `docs:`, `test:`, `refactor:`, `chore:`).
- Keep commit messages descriptive and scoped.
- Avoid mixing refactors, behavior changes, and docs-only updates in one commit when possible.
- Do small, incremental commits that are easy to review, instead of large, monolithic commits.
- Update `CHANGELOG.md` under the unreleased section for user-facing `feat:` and `fix:` changes. Docs-only changes and subtle implementation details are exempt. New entries start with a "Breaking changes" section when applicable, followed by categorized sections (see `CHANGELOG.md` for the current layout). Use a bold inline header for each bullet.

## AI policy

- AI coding agents are allowed to assist with code generation and documentation (for example when dealing with repetitive tasks or boilerplate code). For improved code quality and guidance, tell the AI agent to read this file (`CONTRIBUTING.md`) before making any changes.
- Commit messages with AI assistance should have `Assisted-by: AgentName:ModelVersion` in the footer (for example when using Claude Opus 4.8 on Claude Code, use `Assisted-by: Claude:Opus-4.8`). AI-powered autocomplete is exempt from this requirement.
- Autonomous AI agents opening pull requests (and issues) **aren't allowed**. Any such activity will be detected. This is to make sure an actual human is responsible for code changes, not some automated system.
- Bulk (high volume in short time) AI-generated code and commits **are discouraged**. This is to make sure the code quality remains high and maintainable.
- The repository has an `aislop` CI/CD workflow that detects low-quality AI-generated code (aka "AI slop") using [`npx aislop ci`](https://github.com/scanaislop/aislop).

## Questions and discussion

- Open a GitHub issue for bugs or feature requests.
- For general help and discussion, use the project community channels listed on the [Ferron website](https://ferron.sh/support).
