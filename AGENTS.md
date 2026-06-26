# Repository guidelines

## Project structure

Rust workspace (resolver "2"). Key directories:

- `core/` — runtime foundation: `Module`/`ModuleLoader` traits, config, `Registry`, `Pipeline`, dual `Runtime` (vibeio primary + tokio secondary)
- `bin/` — thin CLI crate, depends on `ferron-entrypoint` with `profile-default` features
- `entrypoint/` — wires all modules; every module crate is an optional feature (see `entrypoint/Cargo.toml`)
- `modules/*` — feature crates grouped as `http-*`, `config-*`, `tls-*`, `dns-*`, `observability-*`, etc.
- `types/*` — shared domain types (`dns`, `http`, `observability`, `ocsp`, `tls`)
- `e2e/` — end-to-end tests via testcontainers (requires Docker + protoc in PATH)
- `docs/` — user-facing docs; sidebar in `docs/docLinks.ts`; synced to separate website repo on push to `3.x`
- `soak/` — soak/chaos tests via Docker Compose
- `doctest/` — standalone harness that runs doc examples against the built binary
- `utils/` — CLI utilities (`fmt`, `kdl2ferron`, `passwd`, `precompress`, `serve`); not in the main server

**Workspace excludes** (in root `Cargo.toml`): `doctest/`, `e2e/`, and `modules/*/fuzz` are **not** members of the main workspace, so `cargo build/test --workspace` skips them. Each has its own `Cargo.toml` and a dedicated run command — see tables below.

## Essential commands

Run from repository root unless noted.

### Build and test

| Command | Purpose |
|---------|---------|
| `cargo build --workspace` | Build all crates |
| `cargo test --workspace` | Unit + inline tests |
| `cargo test -p <crate>` | Single crate |
| `cargo fmt --all --check` | Formatting (no `.rustfmt.toml` — uses defaults) |
| `cargo clippy --workspace --all-targets -- -D warnings` | Lint |
| `cargo shear` | Check unused dependencies (CI) |
| `cargo run --manifest-path doctest/Cargo.toml` | Test doc examples |
| `cd e2e && cargo test` | E2E tests (needs Docker + protoc) |
| `rumdl fmt docs && rumdl check --fix docs` | Lint docs Markdown |

### Run server

```
cargo run -p ferron -- run -c ferron.conf                         # start
cargo run -p ferron -- validate -c ferron.conf                    # validate config
cargo run -p ferron -- validate -c ferron.conf --json             # validate, JSON output
cargo run -p ferron -- doctor -c ferron.conf                      # best-practice check
cargo run -p ferron -- adapt -c ferron.conf                       # dump config as JSON
cargo run -p ferron -- daemon -c ferron.conf --pid-file /path     # Unix daemon
cargo run -p ferron -- winservice install -c ferron.conf          # Windows service install
cargo run -p ferron -- version                                    # version + build info
```

`--config-params key=value;key2=value2` and `--config-adapter <name>` flags are accepted by `run`, `validate`, `doctor`, `adapt`, and `winservice install` (see `entrypoint/src/cli.rs:5`).

### Justfile shortcuts

```
just build                       # cargo build -r
just run                         # cargo run --bin ferron
just prepare-config              # cp configs/ferron.conf.example ferron.conf
just package [target]            # release archive (delegates to packaging/archive)
just package-deb [target]        # Debian package (uses Docker)
just package-rpm [target]        # RPM package (uses Docker)
just package-windows [target]    # Windows installer (Windows host only)
just installer                   # Linux installer (runs `make` in installer/)
just soak [duration] [concurrency]   # Docker Compose, defaults 60m / 50
just chaos [duration] [concurrency]  # Docker Compose, defaults 60m / 50
```

### Fuzzing (requires nightly)

Fuzz crates live under `modules/*/fuzz/` and are excluded from the main workspace. Each needs a separate `cargo +nightly fuzz` invocation from inside the fuzz directory.

```
cargo +nightly fuzz run canonicalize_path           # modules/http-server/fuzz
cargo +nightly fuzz run rate_limit_concurrent       # modules/http-ratelimit/fuzz
```

## Testing structure

Three tiers:
1. **Inline unit tests**: `#[cfg(test)] mod tests` inside source files.
2. **E2E tests**: `e2e/tests/` — each file declared as `[[test]]` in `e2e/Cargo.toml`. Uses `testcontainers` + `reqwest`. Requires Docker daemon + protoc in PATH. Build the test image first: `docker build -f e2e/Dockerfile.test -t e2e-test-ferron:latest .`
3. **Fuzz**: `modules/*/fuzz/` — nightly `cargo-fuzz`. Excluded from main workspace.

Benchmarks in `modules/http-server/benches/` (Criterion, gated on `features = ["bench"]` on the `ferron-http-server` crate).

## Conventions

- **Branch**: all work targets `develop-3.x` (CI workflows filter on it; the 3.x docs site syncs from it).
- **Commits**: Conventional Commits (`feat:`, `fix:`, `docs:`, `test:`, `refactor:`, `chore:`). Update `CHANGELOG.md` under the unreleased section (except docs-only changes).
- **Changelog structure**: New entries use a "Breaking changes" section (when applicable) followed by categorized sections (see `CHANGELOG.md`). Use bold inline headers for each bullet.
- **Config changes**: Update matching pages under `docs/configuration/`. Validate with `cargo run -p ferron -- validate -c ferron.conf`. Docs use sentence-case headings, YAML frontmatter, `ferron` code blocks, and relative links.
- **Module system**: Implement `ModuleLoader` trait. Register stages with `StageConstraint::Before/After` for DAG ordering via `RegistryBuilder`. All trait methods have default no-op impls — override only what's needed.
- **Runtime**: dual model — primary threads run vibeio (one per CPU, pinned, optional io_uring), secondary is tokio.
- **Cross-compilation**: Uses `cross` for Linux targets. `Cross.toml` sets GCC 10 for some targets. `bindgen-cli` required for non-`cross` builds.
- **Docker**: three variants — `Dockerfile` (distroless + musl), `Dockerfile.alpine` (musl), `Dockerfile.debian` (glibc).
- **Invalid configurations**: if intentionally describing invalid configurations, prepend `# INVALID` to exactly the first line of the configuration.

## Documentation principles

- **Describe behavior, not labels**: When documenting features, limitations, or configurations, explain what the system actually does. Prefer explicit, functional descriptions over terminology.
- **Inline callouts over separate notes sections**: Use GFM alert syntax (`> [!note]`, `> [!warning]`, `> [!important]`, `> [!tip]`) for brief callouts inline with the relevant content. Do not use a separate `## Notes and troubleshooting` section at the end of a page.
- **Linters as guidance**: Treat terminology linters (e.g., `woke`) as soft suggestions. Do not let them override clarity, break consistency, or trigger unnecessary diffs.
