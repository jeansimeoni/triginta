<p align="center">
  <img src="docs/assets/triginta-logo.png" alt="Triginta logo" width="384">
</p>

# Triginta

[![CI](https://github.com/jeansimeoni/triginta/actions/workflows/ci.yml/badge.svg)](https://github.com/jeansimeoni/triginta/actions/workflows/ci.yml)
[![Latest Release](https://img.shields.io/github/v/release/jeansimeoni/triginta?include_prereleases&sort=semver)](https://github.com/jeansimeoni/triginta/releases)
[![License: GPL-3.0-only](https://img.shields.io/badge/license-GPL--3.0--only-blue.svg)](LICENSE)

Triginta is a local-first terminal app for Pomodoro tracking and task
management. It supports tasks, projects, tags, filters, timer sessions, and
history, all stored locally in a SQLite database so the app remains fully usable
offline.

Todoist sync is optional. Your data lives locally in SQLite, so the app remains
fully usable even if you never enable sync.

## The Reason

I enjoy personal productivity systems, and Pomodoro has always been one of the
ones that worked best for me. I am also a Todoist user, but because most of my
day-to-day workflow lives in the terminal, constantly switching back and forth
was becoming a hassle.

At the same time, I wanted a practical project to help me start learning Rust.
Triginta is the result.

I originally built it for myself, but decided to make it public in case it is
useful to someone else as well.

## The Name

Triginta is Latin for 30. I am Catholic, I like Latin, and 30 minutes is the
usual combined duration of a focus session plus a short break (25m + 5m).

## Notes

This application is provided with no warranty whatsoever. The first version was
built mainly with Codex in my spare time, and spare time is not exactly abundant
when you are a father of five with a full-time job.

My goal was to get something useful running first and then keep improving it as
I learned more of Rust in practice. I come from a C background, so you may find
some comments in the code where I was mapping new Rust concepts to things I
already knew.

With that in mind, bugs and crashes may happen, feel free to open an issue or
submit a PR and I will do my best to review and improve things along the way.

I also have more features and improvements I want to make. You can find a
probably incomplete list in the [Roadmap](#roadmap) section below.

### Todoist Sync

Todoist sync is entirely optional. The app works on its own with the local
SQLite database, and that local database is the source of truth.

At the moment, Todoist sync is also the easiest way to keep data moving between
different devices. It is an area I have tested in my own day-to-day workflow,
but some quirks and edge cases may still be present.

## Demo

![Triginta screen recording](docs/assets/triginta.gif)

## Core Features

- Pomodoro timer with focus, short break, long break, and session history flows
- Local task management with projects, sections, tags, filters, subtasks, and
  favorites (Todoist style)
- Basic natural-language due date parsing in English, Portuguese, and basic
  Spanish
- Keyboard-first interface
- Configurable timer lengths, themes, glyph/ascii mode, sorting, among other
  things
- Local persistence with no external/online dependencies

## Platform Support

Triginta targets modern Linux and macOS terminals. It should run in any terminal
that supports raw mode and alternate-screen TUI applications.

The default UI uses Nerd Font glyphs. If symbols render incorrectly, set
`ui.glyph_mode = "ascii"` in the config file.

Windows paths are handled by the app directory resolver, but Windows terminal
usage is not a primary release target yet.

## Installation

If you want to install Triginta from source, you will need a Rust toolchain
first. This repository uses [`mise`](https://mise.jdx.dev/) to manage tool
versions, so the simplest path is to install `mise` and let it provision the
required Rust toolchain automatically.

With `mise` installed, here are the steps:

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
Linux `.deb`/`.rpm` packages. Note, however, that these packages come with very
little testing from my end. Please create an issue in case you find problems.

Stable Homebrew and AUR publishing are also available. See
[Install](docs/install.md) for the exact commands, current availability, and
uninstall steps.

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
- `s`, `Space`, or `Enter`: start or resume the timer when the timer panel is
  focused
- `p`: pause the timer
- `D`: open the Donate page in your browser
- `q`: quit

## Donate

If you like this project, you can buy me a beer! It would be really appreciated.

Donations will help fund ongoing Triginta development, releases, maintenance,
and documentation work.

- <a href="https://github.com/sponsors/jeansimeoni"><img src="docs/assets/github-sponsors.svg" alt="GitHub Sponsors" width="18" valign="middle">
  GitHub Sponsors</a>
- <a href="https://www.paypal.com/donate/?business=AVKKMCJ3P77HG&no_recurring=0&item_name=Help+the+development+of+Triginta&currency_code=BRL"><img src="docs/assets/paypal.svg" alt="PayPal" width="18" valign="middle">
  PayPal</a>
- <a href="bitcoin:166SB7XLCgoZM75paAag5XGgjuHTdxFBgY"><img src="docs/assets/bitcoin.svg" alt="Bitcoin" width="18" valign="middle">
  Bitcoin</a> `166SB7XLCgoZM75paAag5XGgjuHTdxFBgY` _BTC network only._

## Roadmap

This section is intentionally incomplete for now. I will expand it as the
project evolves.

- Mouse support where it makes sense
- Better clipboard handling beyond basic terminal copy-paste, including copying
  task titles and other task information more easily
- Task assignment (when syncing with Todoist)
- Task comments
- Calendar view
- More improvements to come

## Acknowledgments

Some projects helped shape what I wanted Triginta to feel like: fast,
keyboard-driven, and comfortable to use from the terminal every day.

They are all excellent projects, and I definitely recommend checking them out if
you appreciate thoughtful, well-designed terminal tools.

- [LazyGit](https://github.com/jesseduffield/lazygit)
- [LazyDocker](https://github.com/jesseduffield/lazydocker)
- [btop++](https://github.com/aristocratos/btop)

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

<p align="center">
  <img src="docs/assets/triginta-icon.png" alt="Triginta logo" width="32">
</p>
