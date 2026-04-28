# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
once public releases begin.

## [Unreleased]

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

## [0.1.0] - Unreleased

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
