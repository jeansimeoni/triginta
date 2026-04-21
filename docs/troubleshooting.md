# Troubleshooting

This page covers common startup, terminal, rendering, data, and logging issues.

## Terminal Raw Mode Errors

Triginta is an interactive TUI. It needs a real terminal with raw-mode support.

If you see an error like `failed to enable raw mode` or `No such device or
address`, start Triginta from an interactive terminal instead of a non-TTY
runner, background job, or command capture.

## Rendering Issues

The default glyph mode expects a Nerd Font. If icons, arrows, or symbols look
wrong, switch to ASCII mode in your config file:

```toml
[ui]
glyph_mode = "ascii"
```

If the layout looks broken:

- Use a terminal with enough width and height.
- Try a common UTF-8 locale such as `en_US.UTF-8`.
- Check whether your terminal font supports the glyphs you selected.
- Use ASCII mode when connecting through limited remote terminals.

## Runtime Dependencies

Linux release artifacts are built as musl targets and are intended not to
require system SQLite or OpenSSL libraries. If a Linux release binary fails with
missing `libsqlite3`, `libssl`, or `libcrypto`, report it as a release artifact
bug.

Triginta still needs a compatible interactive terminal. Font support is a UI
concern only: Nerd Font glyphs are optional, and ASCII mode is the fallback.

## Config File Conflicts

Triginta supports exactly one app config file at a time:

- `config.toml`
- `config.yaml`
- `config.yml`

If more than one exists in the searched config directories, startup fails. Keep
only one active file.

Typical config locations:

- Linux: `~/.config/triginta/config.toml`
- macOS: `~/Library/Application Support/triginta/config.toml`
- macOS also checks: `~/.config/triginta/config.toml`
- Windows: `%APPDATA%\triginta\config.toml`

## Logs

Triginta writes file-based logs under the app data directory.

With default app directories, common locations are:

- Linux: `~/.local/share/triginta/logs/triginta.log`
- macOS: `~/Library/Application Support/triginta/logs/triginta.log`
- Windows: `%APPDATA%\triginta\logs\triginta.log`

With `TRIGINTA_DATA_DIR` set, logs are written to:

```text
$TRIGINTA_DATA_DIR/logs/triginta.log
```

If startup fails before the TUI appears, check this log file first.

## Database Locations

Release builds use `triginta.sqlite3`. Debug builds use `triginta-dbg.sqlite3`.

With default app directories, common release database locations are:

- Linux: `~/.local/share/triginta/triginta.sqlite3`
- macOS: `~/Library/Application Support/triginta/triginta.sqlite3`
- Windows: `%APPDATA%\triginta\triginta.sqlite3`

With `TRIGINTA_DATA_DIR` set, databases are written to:

```text
$TRIGINTA_DATA_DIR/triginta.sqlite3
$TRIGINTA_DATA_DIR/triginta-dbg.sqlite3
```

Do not delete these files unless you intentionally want to remove local task and
session data.

## Isolated Test Runs

Use `TRIGINTA_DATA_DIR` to run Triginta without touching your normal data:

```bash
TRIGINTA_DATA_DIR=/tmp/triginta-isolated triginta
```

For a source checkout:

```bash
TRIGINTA_DATA_DIR=/tmp/triginta-isolated mise exec -- cargo run
```

Remove the isolated directory when finished:

```bash
rm -rf /tmp/triginta-isolated
```

## Todoist Token Issues

Todoist sync is optional. If enabled, the default token source is the
`TRIGINTA_TODOIST_TOKEN` environment variable.

For command-based token loading, Triginta runs the configured program directly.
It does not execute through a shell, so shell syntax, pipes, aliases, and
interpolation are not supported. Put each argument in `token_command.args`.

## Debug-Only Flags

Release builds support only:

```bash
triginta --help
triginta --version
```

Developer-only flags such as `--ascii`, `--short-timer`, `--reset-data`,
`--dry-run-sync`, and `--local-only` are accepted only in debug builds and are
hidden from release help output.
