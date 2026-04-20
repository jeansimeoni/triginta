# Dependency License Review

Reviewed on 2026-04-20 for the Phase 1 switch to `GPL-3.0-only` and
converted into a Phase 2 `cargo-deny` policy.

## Summary

`cargo metadata --format-version 1` resolved 232 packages, including Triginta
itself. No direct or transitive dependency showed an obvious GPLv3-incompatible
license.

Most crates use common permissive Rust ecosystem licenses:

- `MIT`
- `Apache-2.0`
- `MIT OR Apache-2.0`
- `BSD-2-Clause`
- `BSD-3-Clause`
- `ISC`
- `Zlib`
- `Unlicense OR MIT`

## Reviewed Licenses

These licenses or expressions are less common in this dependency graph and
should be carried forward into the Phase 2 `cargo-deny` policy:

- `MPL-2.0`: present via `option-ext`; reviewed as acceptable for a GPLv3
  application dependency.
- `Unicode-3.0`: present via ICU and Unicode support crates; acceptable for
  Unicode data/code dependencies.
- `CDLA-Permissive-2.0`: present via `webpki-roots`; reviewed as a permissive
  data license.
- `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT`: present via several
  low-level target and WASI crates; acceptable because permissive alternatives
  are available.
- `MIT OR Apache-2.0 OR LGPL-2.1-or-later`: present via `r-efi` in the full
  metadata graph; not present in the enforced macOS/Linux `cargo-deny` graph.
- `Apache-2.0 OR BSL-1.0`: present via `ryu`; acceptable because both are
  permissive licenses.
- `Apache-2.0 AND ISC`: present via `ring`; reviewed as permissive terms.

## Phase 2 Follow-Up

Phase 2 enforces this review through `deny.toml`.

Direct dependencies are pinned exactly in `Cargo.toml`; transitive dependencies
are pinned by the committed `Cargo.lock`, including registry checksums. Any
dependency update should be treated as a supply-chain review event.

`cargo-deny` checks the first release targets from the release plan: macOS
x64/ARM64 and Linux x64/ARM64. It denies unknown registries, unknown git
dependencies, yanked crates, unreviewed licenses, wildcard dependencies, and
unreviewed duplicate versions.

The locked graph currently contains reviewed duplicate transitive crates that
are skipped by exact version in `deny.toml`:

- `getrandom@0.2.17` and `getrandom@0.4.2`
- `hashbrown@0.14.5`, `hashbrown@0.15.5`, and `hashbrown@0.16.1`
- `unicode-width@0.1.14` and `unicode-width@0.2.0`

New duplicate versions should fail until reviewed and either removed or added
as explicit version-pinned skips with rationale.
