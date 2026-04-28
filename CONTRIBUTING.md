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

Stable package-manager publishing is layered on top of that release flow:

- Homebrew formulas are published to `jeansimeoni/homebrew-tap`.
- AUR metadata is published to `triginta-bin` on `aur.archlinux.org`.
- Pre-release tags are not published to Homebrew or AUR.

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

- Build and verify release artifacts:

```bash
dist build --artifacts=all
scripts/verify-release-artifacts.sh target/distrib
```

The verification script checks archive contents, Linux musl linkage, and
host-compatible `--version`/`--help` execution. Linux artifacts must not depend
on system `libsqlite3`, `libssl`, or `libcrypto`.

Downloadable `.deb` and `.rpm` packages are attached by the separate `Native
Linux Packages` workflow after a GitHub Release is published. Prefer verifying
that workflow in CI or from disposable containers instead of installing Debian
or RPM packaging tools onto your main development machine.

Local Linux-to-musl artifact builds require the cross-compilation tools reported
by `dist`, such as `cargo-zigbuild` and Zig. If those tools are unavailable
locally, run this artifact build and verification step in release CI or another
prepared build environment.

For macOS artifacts, verify both Intel and Apple Silicon archives on macOS
runners or machines:

```bash
triginta --version
triginta --help
TRIGINTA_DATA_DIR=/tmp/triginta-release-smoke triginta
```

Confirm the app launches in an interactive terminal, creates isolated data/log
files, and exits cleanly.

To cut the release, tag the reviewed release commit and push the tag:

```bash
git tag v0.1.0
git push origin v0.1.0
```

After the `Release` workflow completes, verify that the GitHub Release contains
archives, checksums, the source tarball, and the shell installer. Then wait for
the `Native Linux Packages` workflow to attach the `.deb` and `.rpm` artifacts.
After both workflows succeed, test the installer from the published release
URL, then run the manual `Native Package Smoke` workflow to validate x86_64
`.deb` and `.rpm` installs in Debian and Fedora containers.

Repository settings should protect `v*` tags so only maintainers can create or
move release tags.

Package-manager prerequisites:

- Create the public GitHub repository `jeansimeoni/homebrew-tap`.
- Add `HOMEBREW_TAP_TOKEN` to the `triginta` repository secrets with write
  access to that tap repository.
- Create a dedicated AUR SSH key for GitHub Actions and add the public key to
  your AUR account.
- Add `AUR_SSH_PRIVATE_KEY` and `AUR_KNOWN_HOSTS` to the `triginta` repository
  secrets.
- Optionally set repository variables `AUR_PACKAGER_NAME` and
  `AUR_PACKAGER_EMAIL` to control the Git identity used for AUR commits.

First AUR bootstrap:

1. Generate `PKGBUILD` and `.SRCINFO` from the release `sha256.sum` file with
   `scripts/generate-aur-triginta-bin.sh`.
2. Clone `ssh://aur@aur.archlinux.org/triginta-bin.git`.
3. Copy in the generated `PKGBUILD` and `.SRCINFO`, commit, and push once.

After the first stable release completes:

- Verify Homebrew installation with `brew install jeansimeoni/tap/triginta`.
- Verify AUR installation with `yay -S triginta-bin`.
- If the AUR publish workflow fails after the GitHub Release succeeds, fix the
  packaging issue or secret configuration and rerun the `AUR` workflow with the
  target stable tag and `publish=true`.

Downloadable Linux package artifacts:

- GitHub Releases now include `.deb` and `.rpm` packages built from the
  release-ready Linux musl archives.
- These are direct download artifacts only. Do not document apt or dnf
  repositories until signed repository hosting exists and is tested.

## Private Tooling Policy

Do not commit private local AI tooling artifacts or machine-specific planning notes. Files such as `AGENTS.md`, `.codex/`, and other local assistant folders are ignored and must remain outside public history.
