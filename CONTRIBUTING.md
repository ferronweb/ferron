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
2. Create a branch from `develop-2.x` (this is the default development branch for Ferron 2 work).
3. Make your changes in focused commits.

You can build with the provided helpers (`make` on Unix-like systems, `build.ps1` on Windows).

## Build and check locally

Run from the repository root.

### Rust tests and checks

```bash
cargo test --workspace --verbose
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
```

### Build Ferron using the project workflow

```bash
make build-dev CARGO_FINAL_EXTRA_ARGS="--verbose"
```

Windows:

```powershell
$env:CARGO_FINAL_EXTRA_ARGS = "--verbose"
powershell -ExecutionPolicy Bypass .\build.ps1 BuildDev
```

### Smoke test

After `build-dev`:

```bash
FERRON="$(pwd)/target/debug/ferron" bash smoketest/smoketest.sh
```

Windows:

```powershell
$env:FERRON = $PWD.Path + '\target\debug\ferron'
powershell -ExecutionPolicy Bypass .\smoketest\smoketest.ps1
```

### Docker E2E test suite

If your change affects runtime behavior, networking, modules, container packaging, or configuration parsing, also run:

```bash
bash ./dockertest/test.sh
```

## Documentation expectations

If behavior, configuration, CLI output, installation steps, or defaults change, update documentation in the same pull request.

- Main docs live in `docs/`.
- If you add or rename doc pages, update `docs/docLinks.ts`.
- Keep examples and command snippets aligned with the code and scripts in this repository.

Optional local docs linting/formatting (same tool used in CI):

```bash
rumdl fmt docs
rumdl check --fix docs
```

## Pull request guidelines

- Open pull requests against `develop-2.x` by default.
- Use a clear title and description explaining:
  - what changed
  - why it changed
  - how you validated it (commands you ran)
- Link related issues when applicable.
- Keep pull requests focused; separate unrelated changes.
- Ensure CI is green before requesting review.

## Commit guidance

- Keep commit messages descriptive and scoped.
- Avoid mixing refactors, behavior changes, and docs-only updates in one commit when possible.

## Questions and discussion

- Open a GitHub issue for bugs or feature requests.
- For general help and discussion, use the project community channels listed on the [Ferron website](https://ferron.sh/support).
