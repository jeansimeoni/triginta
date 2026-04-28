#!/usr/bin/env sh
set -eu

repo_root="$(CDPATH='' cd -- "$(dirname "$0")/.." && pwd)"
fixture="$repo_root/packaging/aur/triginta-bin/testdata/sha256.sum"

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT HUP INT TERM

"$repo_root/scripts/generate-aur-triginta-bin.sh" \
    --version 0.1.0 \
    --sha256-file "$fixture" \
    --output-dir "$tmp_dir"

pkgbuild="$tmp_dir/PKGBUILD"
srcinfo="$tmp_dir/.SRCINFO"

[ -f "$pkgbuild" ] || {
    printf 'error: missing PKGBUILD output\n' >&2
    exit 1
}
[ -f "$srcinfo" ] || {
    printf 'error: missing .SRCINFO output\n' >&2
    exit 1
}

grep -Fq "pkgname=triginta-bin" "$pkgbuild"
grep -Fq "pkgver=0.1.0" "$pkgbuild"
grep -Fq "source_x86_64=(\"triginta-\${pkgver}-x86_64.tar.xz::https://github.com/jeansimeoni/triginta/releases/download/v0.1.0/triginta-x86_64-unknown-linux-musl.tar.xz\")" "$pkgbuild"
grep -Fq "source_aarch64=(\"triginta-\${pkgver}-aarch64.tar.xz::https://github.com/jeansimeoni/triginta/releases/download/v0.1.0/triginta-aarch64-unknown-linux-musl.tar.xz\")" "$pkgbuild"
grep -Fq "sha256sums_x86_64=('2222222222222222222222222222222222222222222222222222222222222222')" "$pkgbuild"
grep -Fq "sha256sums_aarch64=('1111111111111111111111111111111111111111111111111111111111111111')" "$pkgbuild"
grep -Fq "provides=('triginta')" "$pkgbuild"
grep -Fq "conflicts=('triginta')" "$pkgbuild"

grep -Fq "pkgbase = triginta-bin" "$srcinfo"
grep -Fq "pkgver = 0.1.0" "$srcinfo"
grep -Fq "source_x86_64 = triginta-0.1.0-x86_64.tar.xz::https://github.com/jeansimeoni/triginta/releases/download/v0.1.0/triginta-x86_64-unknown-linux-musl.tar.xz" "$srcinfo"
grep -Fq "source_aarch64 = triginta-0.1.0-aarch64.tar.xz::https://github.com/jeansimeoni/triginta/releases/download/v0.1.0/triginta-aarch64-unknown-linux-musl.tar.xz" "$srcinfo"
grep -Fq "sha256sums_x86_64 = 2222222222222222222222222222222222222222222222222222222222222222" "$srcinfo"
grep -Fq "sha256sums_aarch64 = 1111111111111111111111111111111111111111111111111111111111111111" "$srcinfo"

printf 'AUR packaging check passed\n'
