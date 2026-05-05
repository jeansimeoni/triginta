# Install

Triginta `0.1.5` is available through GitHub Releases, the shell installer,
Homebrew, the AUR `triginta-bin` package, and source builds.

Project website: <https://triginta.app>

## Requirements

- Linux or macOS terminal with raw-mode TUI support
- `git`
- `mise`
- A C toolchain suitable for building Rust crates with native dependencies

The repository pins Rust through `rust-toolchain.toml` and `mise.toml`.

Prebuilt Linux release artifacts are intended to be self-contained for SQLite
and TLS. Users should not need system SQLite, OpenSSL, `libsqlite3`, `libssl`,
or `libcrypto` to run the published Linux musl binaries.

The UI still requires an interactive terminal. A Nerd Font is optional for
enhanced glyphs; ASCII mode is available for terminals that do not render those
glyphs correctly.

## GitHub Releases

All stable releases are published at:

<https://github.com/jeansimeoni/triginta/releases>

The `v0.1.5` release includes:

- macOS archives for `x86_64` and `aarch64`
- Linux musl archives for `x86_64` and `aarch64`
- `sha256.sum`
- `triginta-installer.sh`

If you prefer a manual archive install, download the asset for your platform,
extract it, and place `triginta` somewhere on your `PATH`.

## Shell Installer

The shell installer downloads the matching release artifact and installs
`triginta` into `${CARGO_HOME:-$HOME/.cargo}/bin`.

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/jeansimeoni/triginta/releases/download/v0.1.5/triginta-installer.sh | sh
```

Confirm the installed binary:

```bash
triginta --version
```

## Homebrew

Install from the maintainer tap:

```bash
brew install jeansimeoni/tap/triginta
```

Upgrade later with:

```bash
brew upgrade jeansimeoni/tap/triginta
```

## AUR And yay

Install the prebuilt binary package from AUR:

```bash
yay -S triginta-bin
```

The AUR package installs the same Linux musl release archives published on
GitHub Releases.

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

Release archives include checksums. Verify downloaded archives against
`sha256.sum` from the GitHub Release.

## Update

If you installed with the shell installer, rerun the installer for the newer
release version.

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

For AUR installs:

```bash
yay -Syu triginta-bin
```

## Uninstall

For a manual local install, remove the binary:

```bash
rm -f ~/.local/bin/triginta
```

To remove the source checkout, delete the cloned repository directory.

For Homebrew, uninstall with:

```bash
brew uninstall triginta
```

For AUR/yay, uninstall with:

```bash
yay -R triginta-bin
```

To remove local app data, delete the platform app data and config directories.
If you used `TRIGINTA_DATA_DIR`, delete that directory instead. Be careful:
this removes the local SQLite database, configuration, themes, and logs stored
there.
