# Triginta

Triginta is a local-first Rust TUI for Pomodoro tracking and task management.

The application is designed to stay fully usable offline. SQLite is the source
of truth, and Todoist sync is being built behind an explicit integration
boundary instead of driving core app behavior.

## Status

Current implementation includes:

- `ratatui` + `crossterm` TUI with multi-panel navigation
- Working Pomodoro timer with focus/break/session history flows
- Local task management with projects, sections, tags, filters, and subtasks
- SQLite bootstrap with first-run empty-state behavior
- Todoist sync foundations:
  - provider boundary with startup + periodic sync triggers
  - local sync state and outbox tables
  - local mutation tracking for syncable entities
  - debounced mutation sync scheduling + adaptive polling
  - outbox retry metadata with backoff scheduling
  - Todoist token configuration via env var or strict command execution
  - outbound Todoist REST transport for create/update/delete operations
- Unit and bootstrap tests around app state, storage, and configuration

Still in progress:

- Live Todoist pull/push with conflict resolution against the remote API

## Tech Stack

- Rust 2024 edition
- `ratatui` for terminal rendering
- `crossterm` for terminal input/output
- `rusqlite` with bundled SQLite
- `chrono` for timestamps
- `tracing` for file-based logging
- `mise` for toolchain management

## Prerequisites

- `mise`
- A working Rust toolchain managed by `mise`

If `mise` is installed, the repository will use the toolchain declared in [`mise.toml`](/home/jeansimeoni/Projects/triginta/mise.toml).

## Getting Started

Install the configured toolchain if needed:

```bash
mise install
```

Build the project:

```bash
mise exec -- cargo build
```

Run the TUI:

```bash
mise exec -- cargo run
```

Force ASCII-only symbols in debug builds:

```bash
mise exec -- cargo run -- --ascii
```

Use short timer durations in debug builds:

```bash
mise exec -- cargo run -- --short-timer
```

Run the test suite:

```bash
mise exec -- cargo test
```

Build an optimized release binary:

```bash
mise exec -- cargo build --release
```

## Running The App

Launch the app with:

```bash
mise exec -- cargo run
```

Core key bindings:

- `Tab`, `l`, or `Right Arrow`: next right-side tab
- `Shift+Tab`, `h`, or `Left Arrow`: previous right-side tab
- `1` through `8`: focus a panel/tab target
- `s`, `Space`, or `Enter`: start or resume the timer when the timer panel is focused
- `p`: pause the timer when the timer panel is focused
- `x` or `Esc`: void the current timer when the timer panel is focused
- `q`: quit

Current layout:

- Left column: timer, daily history, navigation, favorites
- Right column: tasks/details tab or statistics tab

Navigation sidebar tabs:

- `[3]` Navigation
- `[4]` Projects
- `[5]` Tags
- `[6]` Filters

## Data And Logging

On startup, Triginta creates its local directories, initializes SQLite if needed, and writes logs to disk.

By default, application paths are resolved through the platform-specific
standard app directories for `triginta`.

The app also supports overriding the data location with `TRIGINTA_DATA_DIR`.

When `TRIGINTA_DATA_DIR` is set, Triginta uses this layout:

- Data directory: `$TRIGINTA_DATA_DIR`
- Config directory: `$TRIGINTA_DATA_DIR/config`
- App config file: `$TRIGINTA_DATA_DIR/config/config.toml` or YAML equivalent
- SQLite database (release builds): `$TRIGINTA_DATA_DIR/triginta.sqlite3`
- SQLite database (debug builds): `$TRIGINTA_DATA_DIR/triginta-dbg.sqlite3`
- Log file: `$TRIGINTA_DATA_DIR/logs/triginta.log`

Example:

```bash
TRIGINTA_DATA_DIR=/tmp/triginta-dev mise exec -- cargo run
```

This is useful for local development and for keeping test data isolated from your normal app state.

## Configuration

Triginta uses a single application configuration file with sectioned settings
such as `ui`, `timer`, `stats`, and `integrations.todoist`.

Supported formats:

- `config.toml`
- `config.yaml`
- `config.yml`

Documentation:

- [Configuration Guide](/home/jeansimeoni/Projects/triginta/docs/configuration.md)

Themes:

- `ui.theme = "catppuccin-mocha"` is the default
- built-in themes currently include the Catppuccin variants
- custom themes can be added as `toml` or `yaml` files under the per-user
  `themes/` directory inside the app config directory

Debug-only overrides:

- `--ascii`: force ASCII glyphs regardless of config
- `--short-timer`: force `30s/10s/20s` timer lengths for testing
- `--reset-data`: delete local debug SQLite data files before startup
- `--dry-run-sync`: run sync cycles in preview mode without writing to Todoist

## Project Layout

```text
src/
  app/           application state and event loop
  config/        path resolution and tracing setup
  domain/        core domain types
  integrations/  sync provider boundaries
  storage/       SQLite bootstrap and repositories
  ui/            ratatui rendering
tests/
  bootstrap.rs   fresh-install database bootstrap coverage
```

## Testing

The current tests focus on the parts that matter for a stable local-first base:

- App state transitions between screens
- Quit behavior
- In-memory database bootstrap
- Fresh database file creation on disk

Run them with:

```bash
mise exec -- cargo test
```

## Development Notes

- The repository is intentionally a single Cargo package for now.
- SQLite remains the local source of truth.
- Empty-state behavior is intentional and should continue to work on a fresh database.
- Outbound Todoist transport is wired; downstream remote-to-local apply/merge is still in progress.

## License

MIT
