# Install

Triginta is currently distributed from source. Package-manager and shell
installer entries are listed here so users know their status, but they are not
published yet.

## Requirements

- Linux or macOS terminal with raw-mode TUI support
- `git`
- `mise`
- A C toolchain suitable for building Rust crates with native dependencies

The repository pins Rust through `rust-toolchain.toml` and `mise.toml`.

## Build From Source

Clone the repository and build an optimized binary:

```bash
git clone https://github.com/jeansimeoni/triginta.git
cd triginta
mise install
mise exec -- cargo build --release
```

Run the binary directly:

```bash
./target/release/triginta
```

Check the version:

```bash
./target/release/triginta --version
```

## Manual Local Install

After building from source, copy the binary into a directory on your `PATH`:

```bash
install -Dm755 target/release/triginta ~/.local/bin/triginta
```

Confirm that your shell finds the installed binary:

```bash
triginta --version
```

If `~/.local/bin` is not on your `PATH`, add it in your shell configuration.

## Shell Installer

A shell installer is not published yet. Until it exists, use the source or
manual local install methods above.

## Homebrew

A Homebrew formula is not published yet. Until it exists, use the source or
manual local install methods above.

## AUR And yay

An Arch User Repository package is not published yet. Until it exists, use the
source or manual local install methods above.

## Update

For a source checkout:

```bash
git pull
mise install
mise exec -- cargo build --release
```

If you manually installed the binary, copy the rebuilt binary again:

```bash
install -Dm755 target/release/triginta ~/.local/bin/triginta
```

## Uninstall

For a manual local install, remove the binary:

```bash
rm -f ~/.local/bin/triginta
```

To remove the source checkout, delete the cloned repository directory.

To remove local app data, delete the platform app data and config directories.
If you used `TRIGINTA_DATA_DIR`, delete that directory instead. Be careful:
this removes the local SQLite database, configuration, themes, and logs stored
there.
