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
2. Create a branch from `develop-3.x` (this is the default development branch for Ferron 3 work).
3. Make your changes in focused commits.

## Build and check locally

Run from the repository root.

### Rust tests and checks

```bash
cargo test --workspace --verbose
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
```

### Build Ferron

```bash
cargo build
```

### Docker E2E test suite

If your change affects runtime behavior, networking, modules, container packaging, or configuration parsing, also run:

```bash
docker rm -f $(docker ps -a --filter ancestor=e2e-test-ferron -q)
docker image rm e2e-test-ferron
cd e2e
cargo test
```

## Documentation expectations

If behavior, configuration, CLI output, installation steps, or defaults change, update documentation in the same pull request.

- Main docs live in `docs/`.
- If you add or rename doc pages, update `docs/links.json`.
- Keep examples and command snippets aligned with the code and scripts in this repository.

Optional local docs linting/formatting (same tool used in CI):

```bash
rumdl fmt docs
rumdl check --fix docs
```

For other guidelines for writing documentation, see [docs/README.md](./docs/README.md).

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

- Commit messages follow Conventional Commits.
- Keep commit messages descriptive and scoped.
- Avoid mixing refactors, behavior changes, and docs-only updates in one commit when possible.

## AI policy

- AI coding agents are allowed to assist with code generation and documentation (for example when dealing with repetitive tasks or boilerplate code). This repository contains [AGENTS.md](./AGENTS.md) (and CLAUDE.md symlink) file for guiding AI coding agents.
- Commit messages with AI assistance should have `Assisted-by: AgentName:ModelVersion` in the footer (for example when using Claude Opus 4.8 on Claude Code, use `Assisted-by: Claude:Opus-4.8`). AI-powered autocomplete is exempt from this requirement.
- Autonomous AI agents opening pull requests (and issues) **aren't allowed**. Any such activity will be detected.
- The repository has an `aislop` CI/CD workflow that detects low-quality AI-generated code (aka "AI slop") using [`npx aislop ci`](https://github.com/scanaislop/aislop).

## Questions and discussion

- Open a GitHub issue for bugs or feature requests.
- For general help and discussion, use the project community channels listed on the [Ferron website](https://ferron.sh/support).
