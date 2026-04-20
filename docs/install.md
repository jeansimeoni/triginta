# Install

Triginta can be built from source today. After the first GitHub Release is
published, the release page will also provide prebuilt archives, checksums, and
a shell installer.

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

The shell installer is available after a GitHub Release exists. For release
`v0.1.0`, the command will be:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/jeansimeoni/triginta/releases/download/v0.1.0/triginta-installer.sh | sh
```

If that release URL does not exist yet, use the source or manual local install
methods above.

Release archives include checksums. Verify downloaded archives against the
corresponding `.sha256` file or `sha256.sum` from the GitHub Release.

## Homebrew

A Homebrew formula is not published yet. Until it exists, use the source or
manual local install methods above.

## AUR And yay

An Arch User Repository package is not published yet. Until it exists, use the
source or manual local install methods above.

## Update

If you installed with the shell installer after a GitHub Release was published,
run the installer for the newer release version.

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
