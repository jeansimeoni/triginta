# `triginta-bin` AUR Packaging

This directory contains the in-repo source-of-truth for the `triginta-bin`
package published to the Arch User Repository.

`triginta-bin` installs the prebuilt Linux musl archives from GitHub Releases.
It does not build from source.

## Files

- `testdata/sha256.sum`: deterministic fixture for local and CI generation checks
- `scripts/generate-aur-triginta-bin.sh`: renders `PKGBUILD` and `.SRCINFO`
- `scripts/check-aur-packaging.sh`: verifies the generator output in normal CI
- `.github/workflows/aur.yml`: validates generated packaging and pushes updates to
  the AUR repository on stable releases

## Manual bootstrap

1. Create an AUR SSH key dedicated to GitHub Actions.
2. Add the public key to your AUR account.
3. Add `AUR_SSH_PRIVATE_KEY` and `AUR_KNOWN_HOSTS` to the `triginta` GitHub
   repository secrets.
4. Generate the package metadata for the first stable release:

```bash
tmp_dir="$(mktemp -d)"
curl -LsSf https://github.com/jeansimeoni/triginta/releases/download/v0.1.0/sha256.sum -o "$tmp_dir/sha256.sum"
scripts/generate-aur-triginta-bin.sh --version 0.1.0 --sha256-file "$tmp_dir/sha256.sum" --output-dir "$tmp_dir"
```

5. Clone `ssh://aur@aur.archlinux.org/triginta-bin.git`, copy in `PKGBUILD` and
   `.SRCINFO`, commit, and push.

After that first push, the `AUR` GitHub workflow can update the package
automatically for later stable releases.
