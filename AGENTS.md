# Repository guidelines

## Project structure

Ferron 3 is a Rust workspace (resolver "2"). Key directories:

- `core/` — runtime foundation: `Module`/`ModuleLoader` traits, config system, `Registry` (typed stages with DAG ordering via `StageConstraint::Before/After`, typed providers), `Pipeline` (ordered stages + inverse ops), dual `Runtime` (vibeio primary + tokio secondary)
- `bin/` — thin CLI crate, depends on `ferron-entrypoint` with `profile-default` features
- `entrypoint/` — wires all modules; every module crate is an optional feature (see `entrypoint/Cargo.toml`)
- `modules/*` — feature crates grouped as `http-*`, `config-*`, `tls-*`, `dns-*`, `observability-*`, etc.
- `types/*` — shared domain types (`dns`, `http`, `observability`, `ocsp`, `tls`)
- `e2e/` — end-to-end tests via testcontainers (requires Docker + protoc in PATH)
- `docs/` — user-facing docs; sidebar in `docs/docLinks.ts`; synced to separate website repo on push to `3.x`
- `soak/` — soak/chaos tests via Docker Compose
- `doctest/` — standalone harness that runs doc examples against the built binary

## Essential commands

Run from repository root unless noted.

### Build and test

| Command | Purpose |
|---------|---------|
| `cargo build --workspace` | Build all crates |
| `cargo test --workspace` | Unit + inline tests |
| `cargo test -p ferron-http-server` | Single crate |
| `cargo fmt --all --check` | Formatting (no `.rustfmt.toml` — uses defaults) |
| `cargo clippy --workspace --all-targets -- -D warnings` | Lint |
| `cargo shear` | Check unused dependencies (used in CI) |
| `cargo run --manifest-path doctest/Cargo.toml` | Test doc examples |
| `cd e2e && cargo test` | E2E tests (needs Docker + protoc) |
| `rumdl fmt docs && rumdl check --fix docs` | Lint docs Markdown |

### Run server

```
cargo run -p ferron -- run -c ferron.conf          # start
cargo run -p ferron -- validate -c ferron.conf      # validate config
cargo run -p ferron -- adapt -c ferron.conf         # config as JSON
cargo run -p ferron -- daemon -c ferron.conf --pid-file /path  # daemon
```

### Justfile shortcuts

```
just build            # cargo build -r
just run              # cargo run --bin ferron
just prepare-config   # cp configs/ferron.conf.example ferron.conf
just package [target]  # release archive
just package-deb / just package-rpm
just soak / just chaos  # Docker Compose based, configurable duration/concurrency
```

### Fuzzing examples (requires nightly)

```
cargo +nightly fuzz run canonicalize_path           # modules/http-server/fuzz
cargo +nightly fuzz run canonicalize_path_routing   # modules/http-server/fuzz
cargo +nightly fuzz run proxy_protocol              # modules/http-server/fuzz
cargo +nightly fuzz run rate_limit_concurrent       # modules/http-ratelimit/fuzz
```

## Testing structure

Three tiers:
1. **Inline unit tests**: `#[cfg(test)] mod tests` inside source files.
2. **E2E tests**: `e2e/tests/` — each file declared as `[[test]]` in `e2e/Cargo.toml`. Uses `testcontainers` + `reqwest`. Requires Docker daemon + protoc in PATH.
3. **Fuzz**: `modules/*/fuzz/` — nightly `cargo-fuzz`.

Benchmarks in `modules/http-server/benches/` (Criterion, gated on `features = ["bench"]`).

## Conventions

- **Commits**: Conventional Commits (`feat:`, `fix:`, `docs:`, `test:`, `refactor:`, `chore:`). Update `CHANGELOG.md` under the unreleased section (except docs-only changes).
- **Changelog structure**: New entries use a "Breaking changes" section (when applicable) followed by categorized sections (e.g., `Modules`, `Reverse proxy & load balancing`, `DNS & ACME`, `HTTP server core`, `Observability & metrics`, `Core runtime`). Use bold inline headers for each bullet to aid scanning.
- **Config changes**: Update matching pages under `docs/configuration/`. Validate with `cargo run -p ferron -- validate -c ferron.conf`. Docs use sentence-case headings, YAML frontmatter, `ferron` code blocks, relative links, and a `## Notes and troubleshooting` section.
- **Module system**: Implement `ModuleLoader` trait. Register stages with `StageConstraint::Before/After` for DAG ordering via `RegistryBuilder`. All trait methods have default no-op impls — override only what's needed.
- **Runtime**: dual model — primary threads run vibeio (one per CPU, pinned, optional io_uring), secondary is tokio.
- **Cross-compilation**: Uses `cross` for Linux targets. `Cross.toml` sets GCC 10 for some targets. `bindgen-cli` required for non-`cross` builds.
- **Docker**: three variants — `Dockerfile` (distroless + musl), `Dockerfile.alpine` (musl), `Dockerfile.debian` (glibc).

### Hash map conventions

Three hashers, each with a specific use case:

| Hasher | When to use | Why |
|--------|-------------|-----|
| **SipHash** (`std::collections::HashMap`) | Config-time / setup-only maps, or maps where keys come from user input | DOS-resistant, safe default |
| **FxBuildHasher** (`rustc_hash::FxBuildHasher`) | Hot-path `DashMap` instances (proxy state, rate limit buckets, abuse tracking, brute force protection, regex caches, template caches) | Fastest for controlled-key concurrent maps; keys are server-internal (upstream names, IPs, config keys) |
| **AHash** (`ahash::AHashMap` / `ahash::RandomState`) | Per-request single-threaded maps where hash quality matters (e.g., cookie parsing in cache store) | Better distribution than FxHash with competitive speed |

Rules:
- **`DashMap::new()` is never correct** for hot paths — it defaults to SipHash. Always use `DashMap::with_hasher(FxBuildHasher)` for performance-sensitive concurrent maps.
- **Config-only maps** (validators, builders, registries) keep the default SipHash — not performance-sensitive.
- When adding a new `DashMap` on a request path, default to `FxBuildHasher`. Add `rustc-hash` to the crate's `[dependencies]` if not already present.
- `AHashMap`/`AHashSet` is preferred over `FxHashMap`/`FxHashSet` for single-threaded maps where distribution quality matters (cache cookies, vary rules, etc.). Add `ahash` to the crate's `[dependencies]` if not already present.

## Documentation principles

- **Describe behavior, not labels**: When documenting features, limitations, or configurations, explain what the system actually does. Prefer explicit, functional descriptions over terminology.
  - **Do:** "No built-in IP address filtering or access restrictions."
  - **Don't:** Use vague mechanism-focused phrasing (e.g. referring only to allow/block lists).
- **Consistency > novelty**: If a term is required by an upstream API, legacy config, or widely adopted standard, keep it. Do not auto-replace terminology across the codebase unless explicitly instructed.
- **Clarity hierarchy**: Functional precision → internal consistency → audience familiarity → terminology trends.
- **Linters as guidance**: Treat terminology linters (e.g., `woke`) as soft suggestions. Do not let them override clarity, break consistency, or trigger unnecessary diffs.
