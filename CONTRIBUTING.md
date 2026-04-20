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
- Keep developer-only CLI flags hidden from help output and unavailable in release builds.

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

Releases are maintainer-controlled and are published by the generated `dist`
workflow from protected version tags such as `v0.1.0`.

Before tagging a release:

- Ensure `Cargo.toml` has the intended version.
- Ensure `CHANGELOG.md` has a heading for that version and includes licensing
  and source-availability notes.
- Run the local quality gates:

```bash
mise exec -- cargo fmt --check
mise exec -- cargo clippy --all-targets -- -D warnings
mise exec -- cargo test --locked
cargo deny check
```

- Verify release automation:

```bash
dist generate --check
dist plan
dist plan --output-format=json --no-local-paths
```

- Confirm the plan includes:
  - `triginta-installer.sh`
  - `sha256.sum`
  - `source.tar.gz`
  - `aarch64-apple-darwin`
  - `x86_64-apple-darwin`
  - `x86_64-unknown-linux-musl`
  - `aarch64-unknown-linux-musl`

To cut the release, tag the reviewed release commit and push the tag:

```bash
git tag v0.1.0
git push origin v0.1.0
```

After the workflow completes, verify that the GitHub Release contains archives,
checksums, the source tarball, and the shell installer. Then test the installer
from the published release URL.

Repository settings should protect `v*` tags so only maintainers can create or
move release tags.

## Private Tooling Policy

Do not commit private local AI tooling artifacts or machine-specific planning notes. Files such as `AGENTS.md`, `.codex/`, and other local assistant folders are ignored and must remain outside public history.
