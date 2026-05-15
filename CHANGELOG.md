# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.6] - 2026-05-15

### Fixed

- Todoist downstream sync now matches the remote inbox to the bootstrapped
  local inbox by `is_inbox`, fixing persistent sync failures for localized
  non-English inbox names.
- Todoist upstream task creation now allows subtasks with a synced parent to
  proceed even when the project mapping has not synced yet.

## [0.1.5] - 2026-05-05

### Fixed

- Todoist task moves and project reparenting now use the correct upstream move
  operations instead of overloading regular update requests.
- Todoist downstream sync now treats deleted remote projects, sections, tags,
  and filters as tombstones, preventing blank or stale sidebar rows after
  remote deletes.
- Release follow-up workflows now resolve stable tags correctly from Release
  runs, release publishing is idempotent when the GitHub release already
  exists, and `cargo-dist` now allows the intentional CI workflow customization
  used by this repository.

## [0.1.4] - 2026-05-05

### Fixed

- Todoist task moves and project reparenting now use the correct upstream move
  operations instead of overloading regular update requests.
- Todoist downstream sync now treats deleted remote projects, sections, tags,
  and filters as tombstones, preventing blank or stale sidebar rows after
  remote deletes.
- Release follow-up workflows now resolve stable tags correctly from Release
  runs, and release publishing is idempotent when the GitHub release already
  exists.

## [0.1.3] - 2026-05-05

### Fixed

- Todoist filter sync now uses the Sync API filter commands instead of invalid
  REST filter endpoints, fixing upstream create/update/delete failures that
  returned `404`.

## [0.1.2] - 2026-05-03

### Fixed

- Native Linux package publishing now triggers from the published GitHub Release
  instead of relying on fragile tag workflow-run metadata.
- Homebrew tap publishing now bootstraps an empty tap repository and no longer
  fails the release when there is nothing new to commit.
- Public installation and packaging metadata now consistently use
  <https://triginta.app> as the project homepage.
- Installation docs no longer claim `.deb` and `.rpm` artifacts for `0.1.0`,
  which shipped without them due to the release workflow failure.

### Added

- GPL v3 project license and public release metadata.
- Security policy for pre-1.0 vulnerability reporting.
- Homebrew tap publishing configuration for stable releases.
- AUR packaging automation and maintainer bootstrap workflow for `triginta-bin`.
- Native Linux `.deb` and `.rpm` packaging workflows attached to GitHub Releases.
- Debian and Fedora container smoke-test workflow for downloadable native Linux packages.

### Changed

- Package license metadata changed from MIT to `GPL-3.0-only`.
- Installation docs and release runbooks now cover shell, Homebrew, AUR, and downloadable native Linux package flows.

## [0.1.0] - 2026-05-03

### Added

- Local-first Pomodoro timer and task management TUI.
- SQLite-backed local storage with first-run empty-state behavior.
- Projects, sections, tags, filters, subtasks, and session history.
- Todoist sync foundations behind explicit integration boundaries.
- GitHub Release automation for prebuilt Linux/macOS archives, checksums,
  source archives, and a shell installer.
- Downloadable Linux `.deb` and `.rpm` package artifacts built from the verified musl release archives.

### License

- Source and release artifacts are distributed under `GPL-3.0-only`.
- Release archives include the project `LICENSE`, `README.md`, and
  `CHANGELOG.md`.
