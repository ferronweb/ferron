# Repository guidelines

## Project structure

Rust workspace (resolver "2"). Key directories:

- `core/`: runtime foundation: `Module`/`ModuleLoader` traits, config, `Registry`, `Pipeline`, dual `Runtime` (zincio primary + tokio secondary)
- `bin/`: thin CLI crate, depends on `ferron-entrypoint` with `profile-default` features
- `entrypoint/`: wires all modules; every module crate is an optional feature (see `entrypoint/Cargo.toml`)
- `modules/*`: feature crates grouped as `http-*`, `config-*`, `tls-*`, `dns-*`, `observability-*`, etc.
- `types/*`: shared domain types (`dns`, `http`, `observability`, `ocsp`, `tls`)
- `e2e/`: end-to-end tests via testcontainers (requires Docker + protoc in PATH)
- `docs/`: user-facing docs; sidebar in `docs/links.json`; synced to separate website repo on push to `3.x`
- `doctest/`: standalone harness that runs doc examples against the built binary
- `utils/`: CLI utilities (`fmt`, `kdl2ferron`, `passwd`, `precompress`, `serve`); not in the main server

**Workspace excludes** (in root `Cargo.toml`): `doctest/`, `e2e/`, and `fuzz/` are **not** members of the main workspace, so `cargo build/test --workspace` skips them. The `fuzz/` crate has its own `Cargo.toml` and a dedicated run command: see tables below.

## Essential commands

Run from repository root unless noted.

### Build and test

| Command                                                 | Purpose                                                                               |
| ------------------------------------------------------- | ------------------------------------------------------------------------------------- |
| `cargo build --workspace`                               | Build all crates                                                                      |
| `cargo test --workspace`                                | Unit + inline tests                                                                   |
| `cargo test -p <crate>`                                 | Single crate                                                                          |
| `cargo fmt --all --check`                               | Formatting (no `.rustfmt.toml` — uses defaults)                                       |
| `cargo clippy --workspace --all-targets -- -D warnings` | Lint                                                                                  |
| `cargo shear`                                           | Check unused dependencies (CI)                                                        |
| `cargo run --manifest-path doctest/Cargo.toml`          | Test Ferron configurations in doc examples                                            |
| `cd e2e && cargo test`                                  | E2E tests (needs Docker + protoc)                                                     |
| `rumdl fmt docs && rumdl check --fix docs`              | Lint docs Markdown (rumdl needs to be installed)                                      |
| `npx aislop scan`                                       | Scan for possible AI-generated issues (false positives possible though; requires npm) |

### Run server

```
cargo run -p ferron -- run -c ferron.conf                         # start
cargo run -p ferron -- validate -c ferron.conf                    # validate config
cargo run -p ferron -- doctor -c ferron.conf                      # best-practice check
cargo run -p ferron -- version                                    # version + build info
```

See the user-facing documentation in `docs` for details and more commands.

### Justfile shortcuts

This project uses `just` for automating some build tasks (preparing config, packaging, building an installer). See `just --list` for available commands.

### Fuzzing (requires nightly)

All HTTP fuzz targets live under `fuzz/fuzz_targets/` (excluded from the main workspace). Run from inside the `fuzz/` directory (list the files in `fuzz/fuzz_targets/` for available targets):

```
cargo +nightly fuzz run $FUZZ_TARGET
```

Dictionaries and seed corpora are in `fuzz/dictionaries/` and `fuzz/corpus/`.

## Testing structure

Three tiers:

1. **Inline unit tests**: `#[cfg(test)] mod tests` inside source files.
2. **E2E tests**: `e2e/tests/`, each file declared as `[[test]]` in `e2e/Cargo.toml`. Uses `testcontainers` + `reqwest`. Requires Docker daemon + protoc in PATH. Build the test image first: `docker build -f e2e/Dockerfile.test -t e2e-test-ferron:latest .`
3. **Fuzz**: `fuzz/`, nightly `cargo-fuzz`. Excluded from main workspace.

Benchmarks in `modules/http-server/benches/` (Criterion, gated on `features = ["bench"]` on the `ferron-http-server` crate).

## Conventions

- **Branch**: all work targets `develop-3.x` (CI workflows filter on it; the 3.x docs site syncs from it).
- **Commits**: Conventional Commits (`feat:`, `fix:`, `docs:`, `test:`, `refactor:`, `chore:`). Update `CHANGELOG.md` under the unreleased section (except docs-only changes and implementation details, bug fixes and new features are accepted; this is a user-facing changelog). Commit messages should have `Assisted-by: AgentName:ModelVersion` in the footer (for example if you're Claude Opus 4.8 on Claude Code, use `Assisted-by: Claude:Opus-4.8`).
- **Changelog structure**: New entries use a "Breaking changes" section (when applicable) followed by categorized sections (see `CHANGELOG.md`). Use bold inline headers for each bullet.
- **Config changes**: Update matching pages under `docs/configuration/` (validate with `cargo run --manifest-path doctest/Cargo.toml`). Validate with `cargo run -p ferron -- validate -c ferron.conf`. Docs use sentence-case headings, YAML frontmatter, `ferron` code blocks, and relative links. Config files can use either `.conf` or `.ferron` extensions.
- **Stub implementation/known issue comments**: When leaving stubs in the codebase and comments explaining the stubs, include `TODO` markers. For known-issue comments, leave `FIXME` markers.
- **Mandatory updates for features and fixes**: Every `feat:` or `fix:` commit MUST include updates to documentation (under `docs/` or `docs/configuration/`), the changelog (`CHANGELOG.md`, if user-facing as subtle implementation details don't count), and E2E tests (`e2e/tests/`, if applicable) so the change is verified and documentation does not drift. The `docs:` commit type is the exception (it may update documentation alone without adding tests or code).
- **Runtime**: dual model, primary threads run zincio (one per CPU, pinned, optional io_uring), secondary is tokio. When writing HTTP server modules, consider using zincio functions instead of tokio as the code mostly runs on the primary runtime.
- **Cross-compilation**: Uses `cross-build/build.sh` (PGO by default) for Linux targets on Linux hosts and `cross` otherwise. `bindgen-cli` required for non-`cross` builds.
- **Docker**: PGO build images (`Dockerfile` distroless+musl, `Dockerfile.alpine`, `Dockerfile.debian` glibc-slim). No-PGO variants could be built with `--build-arg NOPGO=1`
- **Invalid configurations**: if intentionally describing invalid configurations, prepend `# INVALID` to exactly the first line of the configuration.
- **Idiomatic Ferron 3 configuration style**: When writing `.conf` examples in documentation, follow the new ferronconf spec conventions:
  - **No semicolons**: directives are terminated by newlines, not semicolons.
  - **4-space indentation**: consistent across all examples.
  - **Bare strings preferred**: omit quotes unless the value contains spaces, special characters, or would be ambiguous.
  - **Boolean flags**: write `directive` (bare, no value) when the intent is `true`; write `directive false` only when disabling.
  - **Raw string literals**: use `r"..."` for regex patterns to avoid double-backslash escaping.
  - **Quoted strings**: single and double quotes are interchangeable; use whichever is clearer in context.

## Documentation principles

- **Describe behavior, not labels**: When documenting features, limitations, or configurations, explain what the system actually does. Prefer explicit, functional descriptions over terminology.
- **Callouts**: Use GFM alert syntax (`> [!note]`, `> [!warning]`, `> [!important]`, `> [!tip]`) for brief callouts inline with the relevant content.
- **Linters as guidance**: Treat terminology linters (e.g., `woke`) as soft suggestions. Do not let them override clarity, break consistency, or trigger unnecessary diffs.
- **Documentation scope**: Treat the documentation like a user-facing manual. Do not include internal implementation details or directly re-quote specifications.
- **Short paragraphs over long ones**: Write short, concise paragraphs that are easy to scan, read, and understand. Avoid overly verbose explanations.
- **"Simple English"**: Write in clear, straightforward language that is easy to understand. Use [ASD-STE100](https://asd-ste100.org/) as a reference for writing simplified English.
- **STE rules for docs prose**: Apply these Simplified Technical English rules to all documentation (headings, paragraphs, list items, callout text — not code blocks, inline code, directive names, or URLs):
  - **Active voice**: "Ferron reads the file", not "the file is read by the parser". Use "Ferron" or "the server" as the subject when the actor is the software.
  - **No contractions**: Write "do not", "cannot", "will not", "it is".
  - **Sentence length**: Max 20 words for instructions, max 25 for descriptive sentences. Split longer sentences.
  - **No semicolons**: Use a period and split into two sentences.
  - **Replace banned words**: begin→start, ensure→make sure, utilize→use, "prior to"→before, "subsequent to"→after, obtain→get, demonstrate→show, additionally→also, "in order to"→to, "a variety of"→various, "it is important to note"→delete/restate, "due to the fact that"→because.
  - **No marketing adjectives**: seamless, robust, powerful, effortless, etc.
  - **No nominalizations**: "perform an analysis"→"analyze", "provide documentation"→"document".
  - **No "-ing" main verbs**: "is creating"→"creates", "is running"→"runs".
  - **One topic per paragraph**, max six sentences per paragraph.
  - **No em dashes as separators**: Use a period, comma, or restructure instead. Keep numeric ranges (1-2).
  - **American spelling**.
