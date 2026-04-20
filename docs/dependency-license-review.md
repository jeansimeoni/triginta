# Dependency License Review

Reviewed on 2026-04-20 for the Phase 1 switch to `GPL-3.0-only`.

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
- `MIT OR Apache-2.0 OR LGPL-2.1-or-later`: present via `r-efi`; acceptable
  because permissive alternatives are available.
- `Apache-2.0 OR BSL-1.0`: present via `ryu`; acceptable because both are
  permissive licenses.
- `Apache-2.0 AND ISC`: present via `ring`; reviewed as permissive terms.

## Phase 2 Follow-Up

Phase 2 should convert this review into an automated `cargo-deny` gate with an
explicit allow list and documented exceptions for the less common licenses
above.
