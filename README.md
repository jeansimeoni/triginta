<p align="center">
  <img src="docs/assets/triginta-logo.png" alt="Triginta logo" width="384">
</p>

# Triginta

[![CI](https://github.com/jeansimeoni/triginta/actions/workflows/ci.yml/badge.svg)](https://github.com/jeansimeoni/triginta/actions/workflows/ci.yml)
[![Latest Release](https://img.shields.io/github/v/release/jeansimeoni/triginta?sort=semver)](https://github.com/jeansimeoni/triginta/releases)
[![License: GPL-3.0-only](https://img.shields.io/badge/license-GPL--3.0--only-blue.svg)](LICENSE)

Triginta is a local-first terminal app for Pomodoro tracking and task
management. It keeps your tasks, projects, tags, filters, timer sessions, and
history in a local SQLite database so the app remains useful offline.

Todoist sync support is being built behind explicit integration boundaries, but
local SQLite remains the source of truth.

## Demo

![Triginta screen recording](docs/assets/triginta.gif)

The full-size logo and app icon live under `docs/assets/` for README, release,
and website usage.

## Features

- Pomodoro timer with focus, short break, long break, and session history flows
- Local task management with projects, sections, tags, filters, subtasks, and favorites
- Keyboard-first `ratatui` interface with in-app help via `?`
- Configurable timer lengths, themes, glyph mode, sorting, and Todoist token source
- Local SQLite persistence with empty first-run behavior
- File-based logs for troubleshooting

## Platform Support

Triginta targets modern Linux and macOS terminals. It should run in any terminal
that supports raw mode and alternate-screen TUI applications.

The default UI uses Nerd Font glyphs. If symbols render incorrectly, set
`ui.glyph_mode = "ascii"` in the config file.

Windows paths are handled by the app directory resolver, but Windows terminal
usage is not a primary release target yet.

## Install

The currently available install paths are source-based:

```bash
git clone https://github.com/jeansimeoni/triginta.git
cd triginta
mise install
mise exec -- cargo build --release
./target/release/triginta
```

For manual binary placement after building from source:

```bash
install -Dm755 target/release/triginta ~/.local/bin/triginta
triginta --version
```

GitHub Releases provide prebuilt archives, a shell installer, and downloadable
Linux `.deb`/`.rpm` packages. Stable Homebrew and AUR publishing are wired into
the release process but require maintainer bootstrap before those
package-manager commands become available. See [Install](docs/install.md) for
the exact commands, current availability, and uninstall steps.

## Quick Start

Launch Triginta:

```bash
triginta
```

If you are running from a checkout instead of an installed binary:

```bash
mise exec -- cargo run
```

Useful commands:

```bash
triginta --help
triginta --version
```

Core keys:

- `1-8`: focus a panel
- `Tab` / `Shift+Tab`: move focus
- `?`: open keyboard help
- `c`: create a task
- `s`, `Space`, or `Enter`: start or resume the timer when the timer panel is focused
- `p`: pause the timer
- `D`: open the Donate page in your browser
- `q`: quit

## Donate

Donations help fund ongoing Triginta development, releases, maintenance, and
documentation work.

- ![GitHub Sponsors](https://img.shields.io/badge/GitHub%20Sponsors-EA4AAA?logo=githubsponsors&logoColor=white) [GitHub Sponsors](https://github.com/sponsors/jeansimeoni)
- ![PayPal](https://img.shields.io/badge/PayPal-003087?logo=paypal&logoColor=white) [PayPal](https://www.paypal.com/donate/?business=AVKKMCJ3P77HG&no_recurring=0&item_name=Help+the+development+of+Triginta&currency_code=BRL)
- ![Bitcoin](https://img.shields.io/badge/Bitcoin-F7931A?logo=bitcoin&logoColor=white) [Bitcoin](bitcoin:166SB7XLCgoZM75paAag5XGgjuHTdxFBgY) `166SB7XLCgoZM75paAag5XGgjuHTdxFBgY` _BTC network only._

## Documentation

- [Install](docs/install.md)
- [User Guide](docs/user-guide.md)
- [Configuration](docs/configuration.md)
- [Troubleshooting](docs/troubleshooting.md)
- [NLP Locale Packs](docs/nlp-locales.md)

## Data And Logs

Triginta stores data in platform-standard app directories by default. You can
isolate a run with `TRIGINTA_DATA_DIR`:

```bash
TRIGINTA_DATA_DIR=/tmp/triginta-test triginta
```

With `TRIGINTA_DATA_DIR` set, Triginta uses:

- Data directory: `$TRIGINTA_DATA_DIR`
- Config directory: `$TRIGINTA_DATA_DIR/config`
- Release database: `$TRIGINTA_DATA_DIR/triginta.sqlite3`
- Debug database: `$TRIGINTA_DATA_DIR/triginta-dbg.sqlite3`
- Log file: `$TRIGINTA_DATA_DIR/logs/triginta.log`

See [Troubleshooting](docs/troubleshooting.md) for default platform paths and
common terminal issues.

## Development

This repository is a single Rust package pinned to the Rust toolchain in
`rust-toolchain.toml` and `mise.toml`.

Run the release quality gates locally:

```bash
mise exec -- cargo fmt --check
mise exec -- cargo clippy --all-targets -- -D warnings
mise exec -- cargo test --locked
cargo deny check
```

## License

Triginta is licensed under the GNU General Public License version 3 only
(`GPL-3.0-only`). See [LICENSE](LICENSE).
