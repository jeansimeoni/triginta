# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.11] - 2026-10-26

### Fixed

- Completing a Todoist-synced recurring task after its due time, or while
  overdue, no longer skips the next occurrence. Triginta no longer re-sends
  the recurring due string before closing the task, so Todoist advances the
  recurrence exactly once.

### Security

- Bumped `rustls-webpki` to `0.103.16` and `webpki-roots` to `1.0.11`
  (pulled in transitively via `ureq`, Triginta's Todoist sync HTTP client),
  resolving RUSTSEC-2026-0382 and RUSTSEC-2026-0381.

## [0.1.10] - 2026-10-15

### Added

- Todoist task duration now syncs both ways and is modeled end-to-end: it
  is read from and written to Todoist's `duration`/`duration_unit` task
  fields, persisted locally, and shown as a time range (e.g. `07:00-07:30`)
  in the task list and editors.
- Triginta's own due-date quick-add and editor can now parse a recurring
  due phrase that also carries a duration, such as "every weekday at 07:00
  for 30 minutes", "every wednesday from 8am to 8:30am", or
  `07:00-07:30`-style compact ranges, in English, Portuguese, Spanish, and
  phrases mixing those languages.

### Fixed

- Recurring Todoist tasks no longer lose their recurrence when edited in
  Triginta and synced back. The outbound sync previously re-derived
  whether a task was recurring by re-parsing its due string locally;
  a due string Triginta's parser didn't fully understand (for example one
  carrying a duration suffix) was misclassified as non-recurring and sent
  to Todoist as a one-off date, silently destroying the recurrence.
  Triginta now trusts the already-known-correct recurring flag directly.
- Portuguese "às" and Spanish "a las"/"a la" due-time markers (e.g. "toda
  quarta-feira às 8h") were silently never recognized due to an overly
  strict boundary check; due times in these phrasings now parse correctly.
- The history panel ("Today" pomodoro list, weekly stats, and 30-day focus
  summaries) no longer goes stale in long-running sessions left open across
  a day boundary. The data now refreshes automatically when the local date
  rolls over.

### Security

- Bumped `rustls` (pulled in transitively via `ureq`, Triginta's Todoist
  sync HTTP client) to `0.23.46`, resolving RUSTSEC-2026-0285,
  RUSTSEC-2026-0346, RUSTSEC-2026-0348, and RUSTSEC-2026-0349.

## [0.1.9] - 2026-05-20

### Added

- A user-facing Todoist full resync recovery path:
  `triginta --todoist-full-resync` from the CLI, or `Shift+R` from the sync
  status panel in the TUI.

### Fixed

- Todoist users can now recover from stale incremental sync state without
  manual SQLite edits by resetting the stored downstream sync cursor and
  forcing a full refresh.

## [0.1.8] - 2026-05-20

### Fixed

- Todoist downstream sync now keeps remote-applied entities aligned to the same
  sync timestamp used for `synced_at`, preventing later remote task updates
  from being skipped as falsely dirty local changes.
- Todoist downstream sync now merges duplicate local inbox rows into the
  canonical mapped inbox when a remote inbox project is applied, preventing
  tasks from staying attached to an unmapped local inbox row.
- Database bootstrap now reconciles previously split inbox rows on upgrade, so
  affected users recover on `Database::open()` without waiting for another
  Todoist inbox payload.

## [0.1.7] - 2026-05-15

### Fixed

- Todoist downstream sync now merges duplicate local inbox rows into the
  canonical mapped inbox when a remote inbox project is applied, preventing
  tasks from staying attached to an unmapped local inbox row.
- Database bootstrap now reconciles previously split inbox rows on upgrade, so
  affected users recover on `Database::open()` without waiting for another
  Todoist inbox payload.

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
