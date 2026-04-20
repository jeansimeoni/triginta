# Contributing to Triginta

Thanks for contributing to Triginta.

## Development Setup

1. Install `mise`.
2. Install the pinned dependency policy tool:

```bash
cargo install --locked cargo-deny --version 0.19.4
```

3. From the repository root, run commands with `mise exec -- ...` if your shell has not already loaded the toolchain.
4. Build locally:

```bash
mise exec -- cargo build
```

## Testing and Quality Checks

Run these before opening a pull request:

```bash
mise exec -- cargo fmt --check
mise exec -- cargo clippy --all-targets -- -D warnings
mise exec -- cargo test --locked
cargo deny check
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
- Wait for Code Owner review before merging. Release authority and final merge decisions remain with Jean Simeoni.
- Keep history linear by rebasing or updating your branch before merge when requested.

## Licensing

By submitting contributions, you agree that your work is licensed under the project license and can be redistributed under those terms.

Dependency changes must preserve the GPLv3-compatible dependency policy in
`deny.toml`. Runtime and development dependencies are pinned exactly in
`Cargo.toml`, and transitive dependencies are pinned by the committed
`Cargo.lock`. Treat any `Cargo.toml` or `Cargo.lock` dependency update as a
reviewed supply-chain change: check the new license, advisory status, source,
and duplicate-version impact before merging.

## Release Process

Triginta is preparing for its first public release. Until release automation is
in place, releases are maintainer-controlled and should not be cut from
contributor branches. Future release automation will use `cargo-dist`; release
PRs should document generated artifacts, package metadata changes, and any
changes to supported platforms or installation methods.

## Private Tooling Policy

Do not commit private local AI tooling artifacts or machine-specific planning notes. Files such as `AGENTS.md`, `.codex/`, and other local assistant folders are ignored and must remain outside public history.
