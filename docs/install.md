# Install

Triginta can be built from source today. GitHub Releases provide prebuilt
archives, checksums, and a shell installer. Stable Homebrew and AUR publishing
are configured from the same release artifacts once the maintainer-side tap and
AUR bootstrap steps are complete.

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

## Downloadable .deb And .rpm Packages

GitHub Releases also include downloadable Linux packages built from the same
musl release archives used by the shell installer.

On Debian, Ubuntu, or compatible systems, install the matching `.deb` file:

```bash
sudo dpkg -i triginta_VERSION_ARCH.deb
```

On Fedora, RHEL, or other RPM-based systems, install the matching `.rpm` file:

```bash
sudo dnf install ./triginta-VERSION-RELEASE.ARCH.rpm
```

These are standalone downloadable packages, not apt or dnf repository feeds.
There is no signed apt or dnf repository yet.

## Homebrew

Stable releases can be published to Homebrew through
`jeansimeoni/homebrew-tap`. Once that tap has been bootstrapped and a stable
release has completed successfully, install with:

```bash
brew install jeansimeoni/tap/triginta
```

Homebrew publication is stable-only. Pre-release tags such as `-rc` are not
published to the tap.

If the formula is not available yet, use the shell installer, source build, or
manual local install methods above.

## AUR And yay

Stable releases can also publish `triginta-bin` to the Arch User Repository.
Once the package has been bootstrapped on AUR and the publish workflow has run
successfully, install with:

```bash
yay -S triginta-bin
```

The AUR package installs the prebuilt Linux musl archive from the matching
GitHub Release. Pre-release tags are not published automatically.

If `triginta-bin` is not available yet, use the shell installer, source build,
or manual local install methods above.

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

For Homebrew, uninstall with:

```bash
brew uninstall triginta
```

For AUR/yay, uninstall with:

```bash
yay -R triginta-bin
```

For `.deb` installs, uninstall with:

```bash
sudo apt remove triginta
```

For `.rpm` installs, uninstall with:

```bash
sudo dnf remove triginta
```

To remove local app data, delete the platform app data and config directories.
If you used `TRIGINTA_DATA_DIR`, delete that directory instead. Be careful:
this removes the local SQLite database, configuration, themes, and logs stored
there.
