# Contributing to Triginta

Thanks for contributing to Triginta.

## Development Setup

1. Install `mise`.
2. From the repository root, run commands with `mise exec -- ...` if your shell has not already loaded the toolchain.
3. Build locally:

```bash
mise exec -- cargo build
```

## Testing and Quality Checks

Run these before opening a pull request:

```bash
mise exec -- cargo fmt --check
mise exec -- cargo clippy --all-targets -- -D warnings
mise exec -- cargo test --locked
```

## Project Expectations

- Keep the application local-first and fully usable offline.
- Preserve clear boundaries between app state, UI, domain logic, storage, and integrations.
- Prefer simple, readable Rust over clever abstractions.
- Keep first-run behavior valid with an empty database (no sample data injection).

## Pull Request Expectations

- Use focused PRs with clear commit messages.
- Include tests for behavior changes, especially app state transitions and storage behavior.
- Update docs when configuration or user-visible behavior changes, including `docs/configuration.md` when applicable.

## Licensing

By submitting contributions, you agree that your work is licensed under the project license and can be redistributed under those terms.

## Private Tooling Policy

Do not commit private local AI tooling artifacts or machine-specific planning notes. Files such as `AGENTS.md`, `.codex/`, and other local assistant folders are ignored and must remain outside public history.
